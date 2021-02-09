/// A message writing into a preallocated buffer
pub(crate) struct Message<'a> {
    code: &'a mut u8,
    tail: &'a mut [u8],
    written: usize,
    option: Option<u16>,
    /// Start and end of the (allocated / trimmed) payload
    ///
    /// Keeping this along with written is redundant to an extent, and rather a quick hack from
    /// impleemnting MutableWritableMessage than a good design decision.
    payload_range: Option<(usize, usize)>,
}

impl<'a> Message<'a> {
    pub(crate) fn new(code: &'a mut u8, tail: &'a mut [u8]) -> Self {
        Message { code, tail, written: 0, option: Some(0), payload_range: None }
    }

    /// Return the number of bytes that wee populated inside tail
    pub(crate) fn finish(self) -> usize {
        self.written
    }
}

impl<'a> coap_message::MinimalWritableMessage for Message<'a> {
    type Code = u8;
    type OptionNumber = u16;

    fn set_code(&mut self, code: u8) {
        *self.code = code;
    }

    fn add_option(&mut self, number: u16, data: &[u8]) {
        let delta = number.checked_sub(self.option.take()
                .expect("Sequence violation: Content set"))
            .expect("Sequence violation: Options shuffled");
        self.option = Some(number);
        let encoded = coap_message_utils::option_extension::encode_extensions(delta, data.len() as u16);
        let encoded = encoded.as_ref();
        self.tail[self.written..self.written + encoded.len()].copy_from_slice(encoded);
        self.written += encoded.len();
        self.tail[self.written..self.written + data.len()].copy_from_slice(data);
        self.written += data.len();
    }

    fn set_payload(&mut self, payload: &[u8]) {
        let _ = self.option.take().expect("Sequence violation: doublepayload");
        if payload.len() > 0 {
            self.payload_range = Some((self.written + 1, self.written + 1 + payload.len()));
            self.tail[self.written] = 0xff;
            self.tail[self.written + 1..self.written + 1 + payload.len()].copy_from_slice(payload);
            self.written += 1 + payload.len();
        }
    }
}

impl<'a> coap_message::MutableWritableMessage for Message<'a> {
    fn available_space(&self) -> usize {
        self.tail.len() - self.written
    }

    fn payload_mut(&mut self) -> &mut [u8] {
        let (start, end) = self.payload_range
            // We *could* support this, but other libraries don't so let's not perpetuate the
            // deprecated interface
            .expect("Please set payload initially, or use payload_mut_with_len first");
        &mut self.tail[start..end]
    }

    fn payload_mut_with_len(&mut self, len: usize) -> &mut [u8] {
        if self.payload_range.is_none() {
            // TBD: deduplicate with set_payload (option check could be laxer because it's
            // obviously a first time the payload is touched, but it still does need to clear the
            // option field)
            let _ = self.option.take().expect("Sequence violation: doublepayload");
            if len > 0 {
                self.payload_range = Some((self.written + 1, self.written + 1 + len));
                self.tail[self.written] = 0xff;
                // Not writing any content there
                self.written += 1 + len;
            }
        }
        self.truncate(len);
        // Using the old to implement the new
        #[allow(deprecated)]
        self.payload_mut()
    }

    fn truncate(&mut self, len: usize) {
        let (start, end) = self.payload_range
            .expect("Truncate called before ever setting payload");
        let delta = (end - start).checked_sub(len)
            .expect("Truncating can't make the message longer");
        self.written -= delta;
        self.payload_range = Some((start, start + len));
    }

    fn mutate_options<F>(&mut self, mut f: F) where F: FnMut(u16, &mut [u8]) -> () {
        // TBD this is excessively complex, and grounds for finding a better interface. ("Set
        // option and give me a key to update it later with a mutable reference")?

        // May end in a payload marker or just plain end
        let mut slice = &mut self.tail[..self.written];

        let mut option_base = 0;

        while slice.len() > 0{
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

            // To get take_extension to cooperate...
            let mut readable = &slice[..];

            coap_message_utils::option_extension::take_extension(&mut delta, &mut readable)
                .expect("Invalid encoded option in being-written message");
            coap_message_utils::option_extension::take_extension(&mut len, &mut readable)
                .expect("Invalid encoded option in being-written message");

            // ... and get back to a mutable form
            let new_len = readable.len();
            drop(readable);
            let trim = slice.len() - new_len;
            slice = &mut slice[trim..];

            option_base += delta;

            let len = len.into();
            f(option_base, &mut slice[..len]);
            slice = &mut slice[len..];
        }
    }
}
