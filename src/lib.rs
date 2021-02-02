//! A minimal CoAP server implementation built on [embedded-nal].
//!
//! Caveats:
//!
//! * The server does not perform any amplification mitigation (and the handler can't for lack of
//!   remote information); use this only in controlled environments.
//!
//! * The server does not perform any message deduplication. All handler functions must therefore
//!   be idempotent.
//!
//! * The response logic is implemented using [nb] and does not attempt to store responses for
//!   later invocations. If a request comes in and the response can't be sent right away, it is
//!   discarded.
//!
//! * Being based on embedded-nal, it binds to the any-address but leaves the network stack to
//!   choose the sending address; this leads to subtle bugs when runnign on a system with multiple
//!   IP addresses.
#![no_std]

mod message;

use embedded_nal::nb::Result;

pub fn poll<ST>(stack: &ST, socket: &mut ST::UdpSocket, handler: &mut impl coap_handler::Handler) -> Result<(), ST::Error>
where
    ST: embedded_nal::UdpServer,
{
    // Receive step
    let (extracted, addr, t, msgid, token) = {
        // too bad it needs to be initialized -- but see
        // https://github.com/rust-embedded-community/embedded-nal/issues/12 & co
        let mut buf: [u8; 1152] = [0; 1152];

        let (len, addr) = stack.receive(socket, &mut buf)?;
        let buf = &buf[..len];

        // All the UDP-specific format parsing
        if len < 4 {
            // Ignoring too short a request
            return Ok(());
        }
        let ver = buf[0] >> 6;
        if ver != 1 {
            // Mismatching version: MUST be silently ignored
            return Ok(());
        }
        let t = (buf[0] >> 4) & 0x03;
        if t > 1 {
            // We're not a client and always respond piggy-backed, thus never expect ACKs or RSTs
            return Ok(());
        }
        let tkl = buf[0] & 0x0f;
        let tkl: usize = tkl.into();
        let code = buf[1];
        let msgid = u16::from_be_bytes([buf[2], buf[3]]);

        if len < 4 + tkl {
            // Another form of too short a message
            return Ok(());
        }
        let token = match heapless::Vec::<_, heapless::consts::U8>::from_slice(&buf[4..4 + tkl]) {
            Ok(t) => t,
            // MUST be processed as a message format error -- silently ignoring them.
            _ => return Ok(()),
        };
        let tail = &buf[4 + tkl..];

        let msg = coap_message_utils::inmemory::Message::new(code, tail);

        (
            handler.extract_request_data(&msg),
            addr,
            t,
            msgid,
            token,
        )
    };

    // Send step
    {
        // Could make this smaller based on asking the handler
        let mut buf: [u8; 1152] = [0; 1152];

        buf[0] = (1 << 6) | (if t == 0 { 2 } else { 1 } << 4) | (token.len() as u8);
        buf[2..4].copy_from_slice(&msgid.to_be_bytes());
        buf[4..4 + token.len()].copy_from_slice(&token);
        let (header, tokentail) = buf.split_at_mut(4);
        let code = &mut header[1];
        let tail = &mut tokentail[token.len()..];

        let mut message = message::Message::new(code, tail);
        handler.build_response(&mut message, extracted);
        let written = 4 + token.len() + message.finish();

        // Note that this `?` not only propagates errors but also WouldBlock -- and by doing so
        // discards the incoming request.
        stack.send_to(socket, addr, &mut buf[..written])?;
    };

    Ok(())
}
