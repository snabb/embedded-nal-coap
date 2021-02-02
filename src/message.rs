/// A message writing into a preallocated buffer
pub(crate) struct Message<'a> {
    code: &'a mut u8,
    tail: &'a mut [u8],
    written: usize,
    option: Option<u16>,
}

impl<'a> Message<'a> {
    pub(crate) fn new(code: &'a mut u8, tail: &'a mut [u8]) -> Self {
        Message { code, tail, written: 0, option: Some(0) }
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
            self.tail[self.written] = 0xff;
            self.tail[self.written + 1..self.written + 1 + payload.len()].copy_from_slice(payload);
            self.written += 1 + payload.len();
        }
    }
}

impl<'a> coap_message::MutableWritableMessage for Message<'a> {
    fn available_space(&self) -> usize { todo!() }
    fn payload_mut(&mut self) -> &mut [u8] { todo!() }
    fn truncate(&mut self, _: usize) { todo!() }
    fn mutate_options<F>(&mut self, _: F) where F: FnMut(u16, &mut [u8]) -> () { todo!() }
}
