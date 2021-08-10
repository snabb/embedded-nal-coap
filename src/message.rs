//! This module might be a candidate for moving over to coap-message-utils

#[derive(Copy, Clone)]
enum WriteState {
    /// No payload has been written; the last option had number `latest` and the next free byte is
    /// `end`.
    Options { latest: u16, end: usize },
    /// An empty payload was written, the options end at `end`
    ///
    /// This case is needed because the state can't fall back to `Options` when truncating Payload,
    /// and Payload is better non-empty for the below reasons.
    NoPayload { end: usize },
    /// A non-empty payload has been written, occupying the given slice of the tail
    ///
    /// The non-emptiness is relevant when the payload is later truncated to 0, as then the state
    /// reverts to Options and takes the payload marker away again; if 0 was allowed, it'd need
    /// special-casing to not remove an option byte.
    Payload { start: usize, end: usize },
}

use WriteState::*;

/// A message writing into a preallocated buffer
pub(crate) struct Message<'a> {
    code: &'a mut u8,
    tail: &'a mut [u8],
    cursor: WriteState,
}

impl<'a> Message<'a> {
    pub(crate) fn new(code: &'a mut u8, tail: &'a mut [u8]) -> Self {
        Message {
            code,
            tail,
            cursor: Options { latest: 0, end: 0 },
        }
    }

    /// Last written-to byte
    fn end(&self) -> usize {
        match self.cursor {
            Options { latest: _, end } | Payload { start: _, end } | NoPayload { end } => end,
        }
    }

    /// Return the number of bytes that wee populated inside tail
    pub(crate) fn finish(self) -> usize {
        self.end()
    }
}

impl<'a> coap_message::MinimalWritableMessage for Message<'a> {
    type Code = u8;
    type OptionNumber = u16;

    fn set_code(&mut self, code: u8) {
        *self.code = code;
    }

    fn add_option(&mut self, number: u16, data: &[u8]) {
        let (latest, end) = match &mut self.cursor {
            Options { latest, end } => (latest, end),
            _ => panic!("Sequence violation: Content set"),
        };
        let delta = number
            .checked_sub(*latest)
            .expect("Sequence violation: Options shuffled");
        *latest = number;
        let encoded =
            coap_message_utils::option_extension::encode_extensions(delta, data.len() as u16);
        let encoded = encoded.as_ref();
        self.tail[*end..*end + encoded.len()].copy_from_slice(encoded);
        *end += encoded.len();
        self.tail[*end..*end + data.len()].copy_from_slice(data);
        *end += data.len();
    }

    fn set_payload(&mut self, payload: &[u8]) {
        let optend = match self.cursor {
            Options { latest: _, end } => end,
            // We might allow double setting the payload through later extensions, but as for this
            // interface it's once only. We don't detect double setting of empty payloads, but it's
            // not this implementation's purpose to act as a linter.
            _ => panic!("Sequence violation: doublepayload"),
        };
        if !payload.is_empty() {
            self.tail[optend] = 0xff;
            let start = optend + 1;
            let end = start + payload.len();
            self.tail[start..end].copy_from_slice(payload);
            self.cursor = Payload { start, end };
        }
    }
}

impl<'a> coap_message::MutableWritableMessage for Message<'a> {
    fn available_space(&self) -> usize {
        // This really only makes sense before any payload has been written; probably it is a bad
        // API
        self.tail.len() - self.end()
    }

    fn payload_mut(&mut self) -> &mut [u8] {
        match self.cursor {
            Payload { start, end } => &mut self.tail[start..end],
            // We *could* support this, but other libraries don't so let's not perpetuate the
            // deprecated interface
            _ => panic!("Please set payload initially, or use payload_mut_with_len first"),
        }
    }

    fn payload_mut_with_len(&mut self, len: usize) -> &mut [u8] {
        if len == 0 {
            // Just finish the side effect and return something good enough; this allows the easier
            // path for the rest of the function to pick a start, end, and serve that.
            self.truncate(0);
            return &mut [];
        }

        let (start, end) = match self.cursor {
            // Not checking here whether anything is in range; it'll hit the final borrow and panic
            // there just as well.
            Options { end, .. } | NoPayload { end } => {
                self.tail[end] = 0xff;
                (end + 1, end + 1 + len)
            }
            // We *do* allow growing here -- just because it's easier, not because it's allowed to
            // the client (but we're no linter).
            Payload { start, .. } => (start, start + len),
        };

        let end = end.clamp(0, self.tail.len());

        self.cursor = Payload { start, end };
        &mut self.tail[start..end]
    }

    fn truncate(&mut self, len: usize) {
        self.cursor = match (len, self.cursor) {
            (0, Options { end, .. }) | (0, NoPayload { end, .. }) => NoPayload { end },
            (0, Payload { start, .. }) => NoPayload { end: start - 1 },
            // Is panicking really cheaper here than just writing the payload marker and setting
            // the start right?
            (_, Options { .. }) | (_, NoPayload { .. }) => {
                panic!("Truncating would extend payload")
            }
            // It would also be safe to let this fall through, there's no unsafe access around, and
            // it would just fail when mapped -- but it may be odd because the error is deferred to
            // after finish().
            (_, Payload { start, end }) if end - start < len => {
                panic!("Truncating would extend payload")
            }
            (_, Payload { start, .. }) => Payload {
                start,
                end: start + len,
            },
        }
    }

    fn mutate_options<F>(&mut self, mut f: F)
    where
        F: FnMut(u16, &mut [u8]),
    {
        // TBD this is excessively complex, and grounds for finding a better interface. ("Set
        // option and give me a key to update it later with a mutable reference")?

        let optend = match self.cursor {
            Options { latest: _, end } => end,
            NoPayload { end } => end,
            Payload { start, end: _ } => start - 1, // or start, but why loop just to see a 0xff
        };

        // May end in a payload marker or just plain end
        let mut slice = &mut self.tail[..optend];

        let mut option_base = 0;

        while !slice.is_empty() {
            // This is copied and adapted from
            // coap_messsage_utils::option_iteration::OptPayloadReader and not used through it,
            // because that'd be a whole separate implementation there with mut.
            // (It's bad enough that take_extension needs the trickery)
            let delta_len = slice[0];
            slice = &mut slice[1..];

            if delta_len == 0xff {
                break;
            }

            let mut delta = (delta_len as u16) >> 4;
            let mut len = (delta_len as u16) & 0x0f;

            let new_len = {
                // To get take_extension to cooperate...
                let mut readable = &slice[..];

                coap_message_utils::option_extension::take_extension(&mut delta, &mut readable)
                    .expect("Invalid encoded option in being-written message");
                coap_message_utils::option_extension::take_extension(&mut len, &mut readable)
                    .expect("Invalid encoded option in being-written message");

                readable.len()
            };
            // ... and get back to a mutable form
            let trim = slice.len() - new_len;
            slice = &mut slice[trim..];

            option_base += delta;

            let len = len.into();
            f(option_base, &mut slice[..len]);
            slice = &mut slice[len..];
        }
    }
}
