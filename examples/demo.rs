use embedded_nal::UdpServer;

fn run<S>(stack: &S)
where
    S: UdpServer,
{
    let mut sock = stack.socket()
        .expect("Can't create a socket");

    use coap_handler::implementations::{new_dispatcher, TypedStaticResponse, HandlerBuilder};
    let mut handler = new_dispatcher()
            .at(&[".well-known", "core"], TypedStaticResponse { payload: b"</board>,</led>", contentformat: &[40] })
            .at(&["board"], TypedStaticResponse::from("native"));

    // to be moved into the library
    stack.bind(&mut sock, 5683)
        .expect("Can't bind to port");

    loop {
        match embedded_nal_minimal_coapserver::poll(stack, &mut sock, &mut handler) {
            Err(embedded_nal::nb::Error::WouldBlock) => {
                // Give the CPU some time to not busy-loop. Doing it this way is an experiment to
                // help the discussion of
                // <https://github.com/rust-embedded-community/embedded-nal/issues/47>.
                std::thread::sleep(std::time::Duration::from_millis(50));
            },
            e => e.expect("UDP error during send/receive"),
        }
    }
}

fn main() {
    let stack = &std_embedded_nal::STACK;

    run(stack);
}
