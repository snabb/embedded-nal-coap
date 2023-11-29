use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

const COAP_VERSION: u8 = 1;

#[allow(clippy::upper_case_acronyms)] // because that's how they're written in CoAP
#[derive(FromPrimitive, ToPrimitive, PartialEq, Debug)]
pub(crate) enum Type {
    CON = 0,
    NON = 1,
    ACK = 2,
    RST = 3,
}

#[derive(Debug)]
pub(crate) enum ParseError {
    /// The message is too short that even the most fundamental properties could be determined
    MessageTooShort,
    /// The CoAP version does not match.
    ///
    /// "MUST be silently ignored"
    WrongVersion,
    /// Token was longer than this implementation supports
    ///
    /// This is primarily returned to peers that start going RFC8974 on us, for which we need to
    /// RST to allow them proper autodetection.
    ///
    /// While conceptionally this would need to carry the message ID, this is practically not
    /// needed because the main loop leaves message IDs in place.
    TokenTooLong { msgtype: Type },
}

pub(crate) struct ParsedMessage<'buf> {
    pub msgtype: Type,
    pub token: &'buf [u8],
    pub message: coap_message_utils::inmemory::Message<'buf>,
}

/// Parse a received UDP datagram into a CoAP message
///
/// Nothing after the start of the options is processed;
/// [coap_message_utils::inmemory::OPTION_INVALID] has details on why that works.
///
/// Note that TKL_MAX needs to be at least 8, and that is also a good default value. Larger values
/// only make sense in the context of RFC8974, in which case 32 is a good value.
///
/// while conceptually this would need to return the message ID, this is practically not needed
/// because the main loop leaves message IDs in place.
pub(crate) fn parse<const TKL_MAX: usize>(buf: &[u8]) -> Result<ParsedMessage<'_>, ParseError> {
    use ParseError::*;

    if buf.len() < 4 {
        return Err(MessageTooShort);
    }
    let ver = buf[0] >> 6;
    if ver != COAP_VERSION {
        return Err(WrongVersion);
    }

    let msgtype = Type::from_u8((buf[0] >> 4) & 0x03)
        .expect("Success guaranteed by numberof variants and input masking");

    let tkl = buf[0] & 0x0f;
    let (tkext, tkl) = match tkl {
        0..=12 => (0, tkl.into()),
        // FIXME check if the compiler is smart enough that we can drop the `TKL_MAX` checks -- in
        // theory, it could infer that those branches' outcomes would hit the next `if` anyway, and
        // could thus be collaped into the `_` branch.
        13 if buf.len() >= 5 && TKL_MAX >= 13 => (1, 13 + usize::from(buf[4])),
        14 if buf.len() >= 6 && TKL_MAX >= 269 => (
            2,
            269 + usize::from(u16::from_be_bytes(
                buf[4..6].try_into().expect("Length was pre-checked"),
            )),
        ),
        _ => return Err(TokenTooLong { msgtype }),
    };
    if tkl >= TKL_MAX {
        return Err(TokenTooLong { msgtype });
    }

    let code = buf[1];

    if buf.len() < 4 + tkext + tkl {
        // It's slightly weird that after the option of returning an actionalble error we return an
        // unactionable error again, but this does make sense: it's a weird protocol violation
        // again, sending something with a token length we understand but sending insufficient
        // data.
        // This will need revisiting when we learn to process incomplete messages, although it
        // would be weird to have a receive buffer that doesn't even suffice for a supported token
        // length. (Also, we'll be in a pickle: We can't respond b/c we lost some of the token, and
        // piggy-back responses are probably invalid w/o one, yet "try later" is the apt response,
        // as "RST" gives the wrong impression that we don't support it at all -- so maybe playing
        // dead is the best we can do anyway).
        return Err(MessageTooShort);
    }
    let token = &buf[4 + tkext..][..tkl];
    let tail = &buf[4 + tkext + tkl..];

    Ok(ParsedMessage {
        msgtype,
        token,
        message: coap_message_utils::inmemory::Message::new(code, tail),
    })
}

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

#[cfg(test)]
mod test {
    use super::*;
    use coap_message::ReadableMessage;
    use ParseError::*;
    use Type::*;

    #[test]
    fn test_minimal() {
        let buf = &[0x50, 0, 0, 1];
        let parsed = parse::<8>(buf).unwrap();
        assert!(matches!(parsed.msgtype, NON));
        assert_eq!(parsed.token, b"");
        assert_eq!(parsed.message.code(), 0);
        assert!(parsed.message.options().next().is_none());
    }

    #[test]
    fn test_invalid() {
        let buf = &[0x80, 0, 0, 0];
        assert!(matches!(parse::<8>(buf), Err(WrongVersion)));

        let buf = &[0x40, 0];
        assert!(matches!(parse::<8>(buf), Err(MessageTooShort)));

        // Returning "message too short" would kinda also be in spec
        let buf = &[0x49, 0, 0, 1];
        assert!(matches!(
            parse::<8>(buf),
            Err(TokenTooLong { msgtype: CON })
        ));

        // No matter how large tokens we accept, that's plain invalid
        let buf = &[0x4f, 0, 0, 1];
        assert!(matches!(
            parse::<1000>(buf),
            Err(TokenTooLong { msgtype: CON })
        ));
    }

    #[test]
    fn test_normal_tokens() {
        let buf = &[0x45, 0x44, 1, 2, 0x30, 0x31, 0x32, 0x33, 0x34];
        let parsed = parse::<8>(buf).unwrap();
        assert!(matches!(parsed.msgtype, CON));
        assert_eq!(parsed.token, b"01234");
        assert_eq!(parsed.message.code(), 0x44);
        assert!(parsed.message.options().next().is_none());
    }

    #[test]
    fn test_long_tokens() {
        let buf = &[
            0x49, 0x44, 1, 2, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38,
        ];
        let parsed = parse::<32>(buf).unwrap();
        assert_eq!(parsed.token, b"012345678");

        let buf = &[
            0x4d, 0x44, 1, 2, 0, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a,
            0x3b, 0x3c,
        ];
        let parsed = parse::<32>(buf).unwrap();
        assert_eq!(parsed.token, b"0123456789:;<");

        let buf = &[
            0x4d, 0x44, 1, 2, 1, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a,
            0x3b, 0x3c, 0x3d,
        ];
        let parsed = parse::<32>(buf).unwrap();
        assert_eq!(parsed.token, b"0123456789:;<=");

        let mut buf = [0; 4 + 2 + 269 + 9];
        buf[0] = 0x4e;
        buf[4] = 0;
        buf[5] = 9;
        let parsed = parse::<300>(&buf).unwrap();
        assert_eq!(parsed.token, [0; 269 + 9]);
    }
}
