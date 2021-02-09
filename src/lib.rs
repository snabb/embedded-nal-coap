//! A minimal CoAP server implementation built on [embedded-nal].
//!
//! Usage and operation
//! -------------------
//!
//! Until the project has matured further, see [the example] for usage. The general steps are:
//!
//! [the example]: https://gitlab.com/chrysn/coap-message-demos/-/blob/master/examples/std_embedded_nal_minicoapserver.rs
//!
//! * Get a network stack with a UDP server socket that implments [embedded_nal::UdpServer]
//!
//! * Create a CoAP handler that implements [coap_handler::Handler]; the
//!   [coap_handler::implementations] module contains some building blocks
//!   (including some to combine handlers for individual resources into a handler that picks
//!   sub-handlers from the URI path).
//!
//! * Whenever there is indication that a request might have come in, call [poll] with the stack,
//!   the socket and the handler. This will accept the data from the socket, decode the CoAP
//!   message, pass it to the handler, and send back the response.
//!
//!   It returns successful if a message was processed (or something came in that could be
//!   ignored), propagates out errors from the socket, and returns [nb::Error::WouldBlock] if it
//!   turms out the socket was *not* ready.
//!
//!   By applying the belowmentioned constraints and exercising some of the liberties designed into
//!   CoAP, the server does not need to hold any state of its own.
//!
//! Caveats
//! -------
//!
//! * The server does not perform any amplification mitigation (and the handler can't for lack of
//!   remote information); use this only in environments where this is acceptable (e.g. in closed
//!   networks).
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
//!
//! Messages
//! --------
//!
//! Messages are created right in the socket buffer, and are set out to work with minimal copying
//! to the extent the [embedded_nal] interfaces permit, and prepared to go further once they
//! evolve. In particular, this means that outgoing messages need to be written to in ascenging
//! CoAP option number, in accordance with [coap_message::MinimalWritableMessage] and
//! [coap_message_utils::MutableWritableMessage].
//!
//! This enables this crate to not only be `no_std`, but to not require `alloc` either.
#![no_std]

mod message;

use num_traits::{FromPrimitive, ToPrimitive};
use num_derive::{FromPrimitive, ToPrimitive};

use embedded_nal::nb::Result;

const COAP_VERSION: u8 = 1;

/// Maximum size of a CoAP message we need to expect
///
/// Also used in creating an output buffer as it's allocated the same way anyway.
const MAX_SIZE: usize = 1152;

#[derive(FromPrimitive, ToPrimitive, PartialEq)]
enum Type {
    CON = 0,
    NON = 1,
    ACK = 2,
    RST = 3,
}

/// Attempt to process one message out of the given `socket` on a UDP `stack`.
///
/// Any CoAP requests are dispatched to the handler. A response is built immediately and sent.
///
/// Failure to perform any action immediately makes the function return `WouldBlock`, and it should
/// be called again whenever there is indication that the network device is ready again. Any errors
/// from the stack are propagated out. Errors in message processing (eg. invalid CoAP messages) are
/// treated as per the protocol and are not indicated separately; they cause a successful return.
///
/// Note that the caveats in the module description apply.
pub fn poll<ST>(stack: &ST, socket: &mut ST::UdpSocket, handler: &mut impl coap_handler::Handler) -> Result<(), ST::Error>
where
    ST: embedded_nal::UdpServer + ?Sized,
{
    // Receive step
    let (extracted, addr, t_in, msgid, token) = {
        // too bad it needs to be initialized -- but see
        // https://github.com/rust-embedded-community/embedded-nal/issues/12 & co
        let mut buf: [u8; MAX_SIZE] = [0; MAX_SIZE];

        let (len, addr) = stack.receive(socket, &mut buf)?;
        let buf = &buf[..len];

        // All the UDP-specific format parsing
        if len < 4 {
            // Ignoring too short a request
            return Ok(());
        }
        let ver = buf[0] >> 6;
        if ver != COAP_VERSION {
            // Mismatching version: MUST be silently ignored
            return Ok(());
        }
        let t_in = Type::from_u8((buf[0] >> 4) & 0x03)
            .expect("Success guaranteed by numberof variants and input size");
        if t_in == Type::ACK || t_in == Type::RST {
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
            t_in,
            msgid,
            token,
        )
    };

    // Send step
    {
        // Could make this smaller based on asking the handler
        let mut buf: [u8; MAX_SIZE] = [0; MAX_SIZE];

        let t_out = if t_in == Type::CON { Type::ACK } else { Type::NON };
        buf[0] = (COAP_VERSION << 6) | ((t_out.to_u8().unwrap()) << 4) | (token.len() as u8);
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
