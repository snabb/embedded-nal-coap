//! Tools for reading CoAP-over-UDP messages from a buffer

use crate::udp_format::{Type, COAP_VERSION};

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
    pub msgid: u16,
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
    let msgid = u16::from_be_bytes(buf[2..4].try_into().unwrap());

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
        msgid,
        token,
        message: coap_message_utils::inmemory::Message::new(code, tail),
    })
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::udp_format::Type::*;
    use coap_message::ReadableMessage;
    use ParseError::*;

    #[test]
    fn test_minimal() {
        let buf = &[0x50, 0, 0, 1];
        let parsed = parse::<8>(buf).unwrap();
        assert!(matches!(parsed.msgtype, NON));
        assert_eq!(parsed.token, b"");
        assert_eq!(parsed.message.code(), 0);
        assert_eq!(parsed.msgid, 1);
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
