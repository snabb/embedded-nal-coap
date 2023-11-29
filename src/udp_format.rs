//! Common constants of the CoAP-over-UDP serialization

pub(crate) const COAP_VERSION: u8 = 1;

#[allow(clippy::upper_case_acronyms)] // because that's how they're written in CoAP
#[derive(PartialEq, Debug)]
pub(crate) enum Type {
    CON = 0,
    NON = 1,
    ACK = 2,
    RST = 3,
}

impl Type {
    #[inline(always)]
    pub fn from_u8(val: u8) -> Option<Self> {
        // Not using num_derive FromPrimitive b/c it's hard to use in split out modules
        match val {
            0 => Some(Type::CON),
            1 => Some(Type::NON),
            2 => Some(Type::ACK),
            3 => Some(Type::RST),
            _ => None,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn type_roundtrip() {
        // It's a bit of overkill, but I don't like when I spell out repeated integers like above.
        for i in 0..=255u8 {
            let processed = Type::from_u8(i);
            let back_to_int = processed.map(|i| i as u8);
            if i < 4 {
                assert_eq!(back_to_int, Some(i));
            } else {
                assert_eq!(back_to_int, None);
            }
        }
    }
}
