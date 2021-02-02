use embedded_nal::UdpServer;
use embedded_nal::nb::block;

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
        block!(embedded_nal_minimal_coapserver::poll(stack, &mut sock, &mut handler)).expect("UDP error during send/receive");
    }
}

fn main() {
    let stack = &std_embedded_nal::STACK;

    run(stack);
}
