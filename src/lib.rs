//! A minimal CoAP server implementation built on [embedded_nal].
//!
//! Usage and operation
//! -------------------
//!
//! Until the project has matured further, see [the example] for usage. The general steps are:
//!
//! [the example]: https://gitlab.com/chrysn/coap-message-demos/-/blob/master/examples/std_embedded_nal_minicoapserver.rs
//!
//! * Get a network stack with a UDP server socket that implments [embedded_nal::UdpFullStack]
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
//!   ignored), propagates out errors from the socket, and returns [WouldBlock](embedded_nal::nb::Error::WouldBlock) if it
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
//! * The response logic is implemented using [nb](embedded_nal::nb) and does not attempt to store responses for
//!   later invocations. If a request comes in and the response can't be sent right away, it is
//!   discarded.
//!
//! * Being based on embedded-nal, it binds to the any-address but leaves the network stack to
//!   choose the sending address; this leads to subtle bugs when runnign on a system with multiple
//!   IP addresses.
//!
//! * Messages are created with as little copying as [embedded_nal] permits. For writable messages,
//!   that means that they need to be written to in ascending CoAP option number. This is in
//!   accordance with the implemented [coap_message::MinimalWritableMessage] and
//!   [coap_message::MutableWritableMessage] traits.
//!
//!   That restriction enables this crate to not only be `no_std`, but to not require `alloc`
//!   either.
//!
//! Roadmap
//! -------
//!
//! The goal of this server is to become a component that can be used easily to bring CoAP
//! connectivity to embedded devices at the bare minimum, while still being practically usable.
//!
//! This means that the amplification mitigation topic will need to be addressed, and that security
//! backing must be catered for (probably by referring to an OSCORE/EDHOC mix-in).
//!
//! Other than that, this implementation's plan is to stay simple and utilize the optimizations
//! CoAP offers, even if this means limiting the application (eg. to immediate responses, and to
//! idempotent handlers).
//!
//! The server offers no support for sending requests, and minimal support for receiving responses
//! through [poll_with_response_handler]. That interface is minimal by design and for user
//! friendliness; it is expected to be used by a (necessarily somewhat more stateful) client
//! implementation.
#![no_std]

mod message;

use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use embedded_nal::nb::Result;

const COAP_VERSION: u8 = 1;

/// Maximum size of a CoAP message we need to expect
///
/// Also used in creating an output buffer as it's allocated the same way anyway.
const MAX_SIZE: usize = 1152;

#[allow(clippy::upper_case_acronyms)] // because that's how they're written in CoAP
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
pub fn poll<ST>(
    stack: &mut ST,
    socket: &mut ST::UdpSocket,
    handler: &mut impl coap_handler::Handler,
) -> Result<(), ST::Error>
where
    ST: embedded_nal::UdpFullStack + ?Sized,
{
    fn all_responses_are_unexpected(
        _: u16,
        _: &[u8],
        _: &coap_message_utils::inmemory::Message<'_>,
    ) -> bool {
        false
    }
    poll_with_response_handler(stack, socket, handler, all_responses_are_unexpected)
}

/// Like [poll], but allowing a callback for response messages.
///
/// The `response_handler` will be called on every received CoAP response (including empty ACKs),
/// with message ID, token and the message. It should return true if the response was expected, and
/// false if not; this influences whether a CON will be responded to with an ACK or an RST.
///
/// Users should not rely on the message argument to the handler to be precisely the indicated
/// type; it may change to any implementation of [coap_message::ReadableMessage].
pub fn poll_with_response_handler<ST>(
    stack: &mut ST,
    socket: &mut ST::UdpSocket,
    handler: &mut impl coap_handler::Handler,
    response_handler: impl for<'a> FnOnce(
        u16,
        &'a [u8],
        &'a coap_message_utils::inmemory::Message<'a>,
    ) -> bool,
) -> Result<(), ST::Error>
where
    ST: embedded_nal::UdpFullStack + ?Sized,
{
    // Receive step
    let (extracted, addr, t_in, msgid, token) = {
        // too bad it needs to be initialized -- but see
        // https://github.com/rust-embedded-community/embedded-nal/issues/12 & co
        let mut buf: [u8; MAX_SIZE] = [0; MAX_SIZE];

        let (len, addr) = stack.receive(socket, &mut buf)?;
        let buf = &mut buf[..len];

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

        // Type of empty response to send in non-request cases
        let mut immediate_response = match t_in {
            Type::CON => Some(Type::RST),
            _ => None,
        };

        let msg = coap_message_utils::inmemory::Message::new(code, tail);

        if matches!(
            coap_numbers::code::classify(code),
            coap_numbers::code::Range::Response(_) | coap_numbers::code::Range::Empty
        ) {
            let was_expected = response_handler(msgid, &token, &msg);
            if was_expected && t_in == Type::CON {
                immediate_response = Some(Type::ACK)
            }
        }

        if !matches!(
            coap_numbers::code::classify(code),
            coap_numbers::code::Range::Request
        ) {
            // No high-level processing from the server side, but at least a RST should be set if
            // indicated.
            if let Some(t_response) = immediate_response {
                let empty_tkl = 0;
                buf[0] = (COAP_VERSION << 6) | (t_response.to_u8().unwrap() << 4) | empty_tkl;
                stack.send_to(socket, addr, &buf[..4])?;
            }
            return Ok(());
        }

        if t_in == Type::ACK || t_in == Type::RST {
            // These should never be responses; ignoring them as protocol errors.
            return Ok(());
        }

        (handler.extract_request_data(&msg), addr, t_in, msgid, token)
    };

    // Send step
    {
        // Could make this smaller based on asking the handler
        let mut buf: [u8; MAX_SIZE] = [0; MAX_SIZE];

        let t_out = if t_in == Type::CON {
            Type::ACK
        } else {
            Type::NON
        };
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
        stack.send_to(socket, addr, &buf[..written])?;
    };

    Ok(())
}
