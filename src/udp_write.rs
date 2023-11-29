//! Tools for writing CoAP-over-UDP messages to a buffer
//!
//! This is split in two halves:
//! * [OutgoingResponseMessage] can apply the optimization that it always works on a buffer that
//!   has just been filled with a request, and thus can write the response by leaving message ID
//!   and token in place.
//! * [OutgoingRequestMessage] and the corresponding [finish_outgoing] and [MessageWritten] are
//!   used when writing requests. They can apply the simplification that they don't need to work
//!   for arbitrary token sizes.

use crate::udp_format::{Type, COAP_VERSION};

pub(crate) struct OutgoingResponseMessage<'buf> {
    tkl_including_extra: usize,
    message: coap_message_utils::inmemory_write::Message<'buf>,
}

impl<'buf> OutgoingResponseMessage<'buf> {
    /// Start building an outgoing response in a buffer.
    ///
    /// This assumes that the buffer already contains the request (thus MID and token can be left
    /// in place).
    pub(crate) fn new<const TKL_MAX: usize>(
        buf: &'buf mut [u8],
        msgtype: Type,
        tkl: usize,
    ) -> Self {
        let tkl_including_extra = match tkl {
            269.. if TKL_MAX >= 269 => 2 + tkl,
            13.. if TKL_MAX >= 13 => 1 + tkl,
            _ => tkl,
        };
        // Relying in the first nibble of tkl to be alright already
        buf[0] = ((msgtype as u8) << 4) | buf[0] & 0b1101111;
        let (header, tokentail) = buf.split_at_mut(4);
        Self {
            tkl_including_extra,
            message: coap_message_utils::inmemory_write::Message::new(
                &mut header[1],
                &mut tokentail[tkl_including_extra..],
            ),
        }
    }

    /// Short cut through new and done, which also ensures that the token length is zeroed
    pub(crate) fn empty(buf: &'buf mut [u8], msgtype: Type) -> usize {
        buf[0] = (COAP_VERSION << 6) | ((msgtype as u8) << 4);
        4
    }

    pub(crate) fn message(&mut self) -> &mut coap_message_utils::inmemory_write::Message<'buf> {
        &mut self.message
    }

    pub(crate) fn done(self) -> usize {
        4 + self.tkl_including_extra + self.message.finish()
    }
}

// build_response and the OutgoingRequestMessage implementations could be unified in one if at some point
// we used the ability to send requests with longer tokens; otherwise it's probly easier to keep
// them separate

/// Return type of OutgoingRequestMessage::done
#[derive(Copy, Clone, Debug)]
pub(crate) struct MessageWritten(usize);

/// Helper for writing outgoing requests (used in the user task)
pub(crate) struct OutgoingRequestMessage<'buf>(
    pub(crate) coap_message_utils::inmemory_write::Message<'buf>,
);

impl<'buf> OutgoingRequestMessage<'buf> {
    /// Create a message
    ///
    /// This is the first half of finish_outgoing, ahnd those two share some assumptions (eg. on
    /// the token length) that would otherwise need to be coordinated through FillBuffer in a
    /// 3-part dance.
    pub(crate) fn new(buf: &'buf mut [u8]) -> Self {
        // Token length is 2
        let (header, tail) = buf.split_at_mut(6);
        OutgoingRequestMessage(coap_message_utils::inmemory_write::Message::new(
            &mut header[1],
            tail,
        ))
    }
    pub(crate) fn message(&mut self) -> &mut coap_message_utils::inmemory_write::Message<'buf> {
        &mut self.0
    }
    pub(crate) fn done(self) -> MessageWritten {
        MessageWritten(self.0.finish())
    }
}

/// Helper for writing outgoing requests (used in the runner task). This finalizes the buffer in
/// which an outgoing request has been assembled.
///
/// This is the second half of OutgoingRequestMessage::new, and those two share some assumptions
/// (eg. on the token length) that would otherwise need to be coordinated through FillBuffer in a
/// 3-part dance.
///
/// It returns the number of valid bytes in buffer.
///
/// It assumes that the original OutgoingRequestMessage's input and the buffer are the same; if
/// they are not, the message produced will contain garbage.
pub(crate) fn finish_outgoing(
    buf: &mut [u8],
    confirmable: bool,
    msgid: u16,
    token: u16,
    written: MessageWritten,
) -> usize {
    let tkl = 2;
    let msgtype = if confirmable { Type::CON } else { Type::NON };
    buf[0] = (COAP_VERSION << 6) | ((msgtype as u8) << 4) | tkl;
    buf[2..4].copy_from_slice(&msgid.to_be_bytes());
    buf[4..6].copy_from_slice(&token.to_be_bytes());

    written.0 + 6
}
