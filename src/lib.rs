//! A CoAP server and client implementation built on [embedded_nal_async].
//!
//! Usage and operation
//! -------------------
//!
//! An example of how to use this will be available as part of the [coap-message-demos] crate.
//!
//! * Open a UDP server socket that implements [embedded_nal_async::UnconnectedUdp]
//!
//! * If you want to run a server,
//!   create a CoAP handler that implements [coap_handler::Handler]; the
//!   [coap-handler-implementations] module contains some building blocks
//!   (including some to combine handlers for individual resources into a handler that picks
//!   sub-handlers from the URI path).
//!
//! * If you want to send requests,
//!   FIXME crate some kind of channel.
//!
//! * Pick an async executor suitable for your platform, and run a FIXME run() function as a task,
//!   providing a UDP socket address and the inputs from above.
//!
//!   By applying the belowmentioned constraints and exercising some of the liberties designed into
//!   CoAP, the server does not need to hold any state of its own.
//!   (FIXME revisit)
//!
//! .. [coap-message-demos]: https://crates.io/crates/coap-message-demos
//! .. [coap-handler-implementations]: https://crates.io/crates/coap-handler-implementations
//!
//! Caveats
//! -------
//!
//! * The server does not perform any amplification mitigation (and the handler can't for lack of
//!   remote information); use this only in environments where this is acceptable (e.g. in closed
//!   networks).
//!
//!   FIXME only provide the 3x buffer for responses / when the handler indicates that it needs
//!   more, 4.01 Echo? (The handler may be unhappy that it gets dropped; -handlers may need
//!   guidance on this)
//!
//! * The server does not perform any message deduplication. All handler functions must therefore
//!   be idempotent.
//!
//! * Messages are created with as little copying as [embedded_nal] permits. For writable messages,
//!   that means that they need to be written to in ascending CoAP option number. This is in
//!   accordance with the implemented [coap_message::MinimalWritableMessage] and
//!   [coap_message::MutableWritableMessage] traits.
//!
//!   That restriction enables this crate to not only be `no_std`, but to not require `alloc`
//!   either.
//!
//! Choices
//! -------
//!
//! This implementation of CoAP chooses to go with a single task, thus only ever allocating a
//! single buffer as part of the task. There are certainly alternative choices to be made here,
//! which may either be implemented in a different crate or altered later (for example, if it turns
//! out that a more effective implementation uses different tasks for send and receive, but uses a
//! single buffer or an at-least-1-sized pool that gets locked by the tasks).
#![no_std]

mod message;
mod udp_format;

use udp_format::ParseError::*;
use udp_format::Type;
use udp_format::Type::*;

use coap_message::ReadableMessage;
use embedded_nal_async::SocketAddr;

/// Maximum size of a CoAP message we need to expect
///
/// Also used in creating an output buffer as it's allocated the same way anyway.
const MAX_SIZE: usize = 1152;

// FIXME make configurable
const TKL_MAX: usize = 32;

// If we went with a second buffer to be able to send 5.03s early on, that'd need to be sized to
// the maximum of token length we support

/// Ways to reply to a message
enum Extracted<R> {
    Silent,
    JustRst,
    JustAck,
    ProcessResponseThenAck(usize),
    ProcessResponseThenSilent(usize),
    ReactTo {
        msgtype: Type,
        token: heapless::Vec<u8, TKL_MAX>,
        extracted: R,
    },
}
use Extracted::*;

// Copy and Clone are mainly there because this makes it easier to pass this around while going
// through the state machine of RequestState
#[derive(Copy, Clone, Debug)]
struct RequestMatchingDetails {
    // If we, as an implementation detail, decide never to have pending requests to different
    // remotes, may we just elide this?
    remote: SocketAddr,
    mid: u16,
    token: u16,
}

// Copy and Clone are mainly there because this makes it easier to pass this around while going
// through the state machine
#[derive(Copy, Clone, Debug)]
enum RequestState {
    /// Slot is unused
    Empty,
    /// A client application component places this when it wants to send a request
    ///
    /// FIXME: Would it make sense to also have a state where an ExpectMore can be turned into a
    /// Pending, eg. to actively cancel an observation? (If RequestMatchingDetails chose not to
    /// carry the remote, it'd need to be provided anew, with erroneous input from the application
    /// just resulting in weird messages that are sent)
    AllocateRequest { remote: SocketAddr },
    /// The loop allows this application component to write the buffer, and awaits signalling
    /// through `.app_to_loop`.
    FillBuffer(RequestMatchingDetails),
    /// The application component is done filling the buffer
    BufferFilled {
        details: RequestMatchingDetails,
        len: usize,
    },

    /// State while the loop is awaiting a response
    RequestSent(RequestMatchingDetails),

    /// This is the loop sets once a response has arrived. Once the client is done, it needs to
    /// signal through `.app_to_loop`.
    ResponsePending(RequestMatchingDetails),
    // After that, the client either sets Empty or RequestSent.
}
use RequestState::*;

/// A CoAP main loop, which would typically run as a server and client simultaneously
///
/// Relevant generics:
/// * `Socket` indicates which kind of UDP socket this will run on
/// * `Handler` is the CoAP request handler type -- the application's implementation of a CoAP
///   server
/// * `CONCURRENT_REQUESTS` indicates how many requests may be outstanding simultaneously on the
///   client side. Note that this distinct from `NSTART` in two ways: `NSTART` is counted per peer,
///   and is decremented as soon as the request is ACK'ed.
pub struct CoAPClient<const CONCURRENT_REQUESTS: usize> {
    // This is held by run (even, even mainly, particular across await points) except while it is
    // asking a specific request to act on it.
    buffer: core::cell::RefCell<[u8; MAX_SIZE]>,

    // This can be taken by anybody currently executing on our loop, and is not held across await
    // points.
    requests: core::cell::RefCell<[RequestState; CONCURRENT_REQUESTS]>,
    // After anything but run() updates requests, it signals updated_request so that run can take
    // any signals out of there and let the pending clients do their work.

    // FIXME is NoopRawLoop safe in the presence of any executor?
    app_to_loop: embassy_sync::signal::Signal<embassy_sync::blocking_mutex::raw::NoopRawMutex, ()>,
    // When run() needs anything from the client application, it pings it through this (FIXME:
    // There has to be a better way than MAX_SIZE signals)
    loop_to_app: [embassy_sync::signal::Signal<embassy_sync::blocking_mutex::raw::NoopRawMutex, ()>;
        CONCURRENT_REQUESTS],
}

/// Parts of the `.run()` loop that are exclusively held
struct RunParts<'a, Socket, Handler, RandomSource, const CONCURRENT_REQUESTS: usize>
where
    Socket: embedded_nal_async::UnconnectedUdp + ?Sized,
    Handler: coap_handler::Handler,
    RandomSource: rand_core::RngCore,
{
    shared: &'a CoAPClient<CONCURRENT_REQUESTS>,

    socket: &'a mut Socket,
    handler: &'a mut Handler,

    rand: &'a mut RandomSource,

    // FIXME: There is time stuff involved actually -- not sending 64k messages within short times,
    // and outbound rate limiting. No clue how to do that yet.
    // FIXME: While they were still locals of the loop() function we could imagine they wouldn't
    // even be used when there are no ways to get requests; now, should we do away with their
    // storage when not sending as a client?
    next_mid: u16,
    next_token: u16,
}

impl<const CONCURRENT_REQUESTS: usize> CoAPClient<CONCURRENT_REQUESTS> {
    pub fn new() -> Self {
        // weird array initialization but OK
        const EMPTY_SIGNAL: embassy_sync::signal::Signal<
            embassy_sync::blocking_mutex::raw::NoopRawMutex,
            (),
        > = embassy_sync::signal::Signal::new();
        Self {
            buffer: [0; MAX_SIZE].into(),
            requests: [Empty; CONCURRENT_REQUESTS].into(),
            app_to_loop: Default::default(),
            loop_to_app: [EMPTY_SIGNAL; CONCURRENT_REQUESTS].into(),
        }
    }

    /// FIXME
    ///
    /// If any receive error occurs, this terminates.
    pub async fn run<Socket, Handler, RandomSource>(
        &self,
        socket: &mut Socket,
        handler: &mut Handler,
        rand: &mut RandomSource,
    ) -> Result<(), Socket::Error>
    where
        Socket: embedded_nal_async::UnconnectedUdp + ?Sized,
        Handler: coap_handler::Handler,
        RandomSource: rand_core::RngCore,
    {
        let start_values = rand.next_u32();
        let next_mid = (start_values >> 16) as u16;
        let next_token = start_values as u16;

        let mut run_parts = RunParts {
            shared: &self,
            socket,
            handler,
            next_mid,
            next_token,
            rand,
        };

        run_parts.run().await
    }

    pub fn request_from(
        &self,
        address: SocketAddr,
    ) -> RequestingCoAPClient<'_, CONCURRENT_REQUESTS> {
        RequestingCoAPClient {
            shared: self,
            address,
        }
    }
}

/// The actual [coap_request::Stack] implementation derived from a [CoAPClient] by putting in an
/// address through `.request_from()`.
pub struct RequestingCoAPClient<'a, const CONCURRENT_REQUESTS: usize> {
    address: SocketAddr,
    shared: &'a CoAPClient<CONCURRENT_REQUESTS>,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum TransportError {
    /// An RST was sent in response to the request message
    GotRst,
}

impl<'a, const CONCURRENT_REQUESTS: usize> coap_request::Stack
    for RequestingCoAPClient<'a, CONCURRENT_REQUESTS>
{
    type RequestMessage<'b> = coap_message_utils::inmemory_write::Message<'b> where Self: 'b;
    type ResponseMessage<'b> = coap_message_utils::inmemory::Message<'b> where Self: 'b;
    type TransportError = TransportError;

    async fn request<Req: coap_request::Request<Self>>(
        &mut self,
        mut request: Req,
    ) -> Result<Req::Output, TransportError> {
        // FIXME hardcoded number
        let mut requests = self.shared.requests.borrow_mut();
        requests[0] = AllocateRequest {
            remote: self.address,
        };
        drop(requests);
        self.shared.app_to_loop.signal(());

        self.shared.loop_to_app[0].wait().await;
        let mut requests = self.shared.requests.borrow_mut();
        let FillBuffer(details) = requests[0] else {
            panic!("Lockstep violation");
        };

        let mut buf = self.shared.buffer.borrow_mut();

        // FIXME use token length
        let (mut header, mut tail) = buf.split_at_mut(6);
        let mut message = message::Message::new(&mut header[1], &mut tail);

        let carry = request.build_request(&mut message).await;
        let len = message.finish();

        requests[0] = BufferFilled { details, len };
        drop(buf);
        drop(requests);

        self.shared.app_to_loop.signal(());

        self.shared.loop_to_app[0].wait().await;

        let buf = self.shared.buffer.borrow();
        // FIXME: There is overhead in running this twice. Should we pass indices?
        let Ok((_msgtype, _token, msg)) = udp_format::parse::<TKL_MAX>(&buf[..len]) else {
            unreachable!("Message was parsed once already");
        };

        let response = request.process_response(&msg, carry).await;

        let mut requests = self.shared.requests.borrow_mut();
        requests[0] = Empty;
        self.shared.app_to_loop.signal(());

        Ok(response)
    }
}

impl<'a, Socket, Handler, RandomSource, const CONCURRENT_REQUESTS: usize>
    RunParts<'a, Socket, Handler, RandomSource, CONCURRENT_REQUESTS>
where
    Socket: embedded_nal_async::UnconnectedUdp + ?Sized,
    Handler: coap_handler::Handler,
    RandomSource: rand_core::RngCore,
{
    pub async fn run(&mut self) -> Result<(), Socket::Error> {
        loop {
            let mut buf = self.shared.buffer.borrow_mut();
            // Receive step
            match embassy_futures::select::select(
                self.shared.app_to_loop.wait(),
                self.socket.receive_into(&mut *buf),
            )
            .await
            {
                // app_to_loop signalled
                embassy_futures::select::Either::First(_) => {
                    // Nothing was written in there, but the application will need to write here
                    // soon
                    drop(buf);

                    self.process_app_to_loop().await?;
                }
                // Message was received
                embassy_futures::select::Either::Second(receive_result) => {
                    let (len, local, remote) = receive_result?;

                    let action = match udp_format::parse::<TKL_MAX>(&buf[..len]) {
                        Err(MessageTooShort | WrongVersion) => Silent,
                        Err(TokenTooLong { msgtype: CON }) => JustRst,
                        Err(TokenTooLong { msgtype: _ }) => Silent,
                        Ok((msgtype, token, message)) => {
                            match coap_numbers::code::classify(message.code()) {
                                coap_numbers::code::Range::Response(_)
                                | coap_numbers::code::Range::Empty => {
                                    // FIXME: If the response is coming in on a RST, we should probably not
                                    // even bother the message handler. (Well at least it should cancel
                                    // retransmissions, so it probably *should* go in there).

                                    match (msgtype, self.match_to_slot(&buf[..], &token)) {
                                        (CON, Some(slot)) => ProcessResponseThenAck(slot),
                                        (CON, None) => JustRst,
                                        (_, Some(slot)) => ProcessResponseThenSilent(slot),
                                        (_, None) => Silent,
                                    }
                                }
                                coap_numbers::code::Range::Request => {
                                    let action = if msgtype == ACK || msgtype == RST {
                                        // These should never be responses; ignoring them as protocol errors.
                                        Silent
                                    } else {
                                        ReactTo {
                                            msgtype,
                                            token,
                                            extracted: self.handler.extract_request_data(&message),
                                        }
                                    };
                                    action
                                }
                                _ => Silent,
                            }
                        }
                    };

                    drop(buf);
                    if let ProcessResponseThenAck(slot) | ProcessResponseThenSilent(slot) = &action
                    {
                        self.handle_response(*slot).await;
                    };
                    let mut buf = self.shared.buffer.borrow_mut();

                    // Send step
                    let (responsetype, tkl, written) = match action {
                        ReactTo {
                            msgtype,
                            token,
                            extracted,
                        } => {
                            // FIXME can we even leave the token in place?
                            // buf[4..4 + token.len()].copy_from_slice(&token);
                            let (header, tokentail) = buf.split_at_mut(4);
                            let code = &mut header[1];
                            let tail = &mut tokentail[token.len()..];

                            // FIXME amplification mitigation

                            let mut message = message::Message::new(code, tail);
                            self.handler.build_response(&mut message, extracted);
                            let written = 4 + token.len() + message.finish();
                            let responsetype = match msgtype {
                                CON => ACK,
                                _ => NON,
                            };
                            (responsetype, token.len(), written)
                        }
                        JustRst => {
                            buf[1] = coap_numbers::code::EMPTY;
                            (RST, 0, 4)
                        }
                        JustAck | ProcessResponseThenAck(_) => {
                            buf[1] = coap_numbers::code::EMPTY;
                            (ACK, 0, 4)
                        }
                        Silent | ProcessResponseThenSilent(_) => continue,
                    };
                    // FIXME move to udp_format, and allow longer token lengths
                    let COAP_VERSION = 1;
                    buf[0] = (COAP_VERSION << 6) | ((responsetype as u8) << 4) | (tkl as u8);
                    // buf[2..4] left in place

                    self.socket.send(local, remote, &buf[..written]).await?;
                }
            };
        }
    }

    /// Build the request by signalling and awaiting the sending task, and send it
    async fn send_request(&mut self, slot: usize) -> Result<(), Socket::Error> {
        self.shared.loop_to_app[slot].signal(());
        // Across here we should only have the slot number as a local variable
        self.shared.app_to_loop.wait().await;
        // FIXME: Actually, there could be any reason, plus we'd really need to clear: So
        // we should loop, clearing the signal, while our buffer is not BufferFilled, and
        // then at any rate signal again so that we don't lose signals.

        let mut requests = self.shared.requests.borrow_mut();
        let BufferFilled { details, len } = &requests[slot] else {
            // FIXME: allow it to declare that it has changed its mind
            panic!("Sequence violation by client");
        };

        let mut buf = self.shared.buffer.borrow_mut();
        // Our buffer preparation... FIXME do through the udp_format module
        buf[0] = 0x42;
        buf[2..4].copy_from_slice(&details.mid.to_be_bytes());
        buf[4..6].copy_from_slice(&details.token.to_be_bytes());
        let len = 6 + len;

        // FIXME can this be any-protocol?
        const SOCK_UNSPEC: SocketAddr = SocketAddr::new(
            embedded_nal_async::IpAddr::V6(embedded_nal_async::Ipv6Addr::UNSPECIFIED),
            0,
        );
        // FIXME: Should we allow the requester to do follow-ups from the same address, or
        // will they just not see the change and suffer breakage mid-blockwise?
        let local = SOCK_UNSPEC;
        let remote = details.remote;

        let new_state = RequestSent(*details);
        requests[slot] = new_state;
        drop(requests);

        self.socket.send(local, remote, &buf[..len]).await?;

        Ok(())
    }

    async fn process_app_to_loop(&mut self) -> Result<(), Socket::Error> {
        // We now have to process *any* slots that are due for us to handle, lest we may lose
        // information that was in the `app_to_loop`. It suffices to process any slot once, for
        // even if this is later changed to allow running on multiple threads, the slot will only
        // enter an application processable state after app_to_loop has been cleared, and the new
        // signal will be pending on the next run of the main loop.

        for slot in 0..CONCURRENT_REQUESTS {
            // At least as long as this is based on a Mutex that is not Send, there should be no
            // need for any optimizations that avoid the drop-and-borrow across the loop (for the
            // compiler can do them).
            let mut requests = self.shared.requests.borrow_mut();

            // That's the only relevant state to consider here. The other states an app can state and
            // then signal (BufferFilled, and the RequestSent or Empty after reading a response) are
            // processed at different places (down here, and in the receive position, respectively).
            if let AllocateRequest { remote } = &mut requests[slot] {
                self.next_mid += 1;
                self.next_token += 1;
                requests[slot] = FillBuffer(RequestMatchingDetails {
                    mid: self.next_mid,
                    token: self.next_token,
                    remote: *remote,
                });
                drop(requests);

                self.send_request(slot).await?;
            }
        }
        Ok(())
    }

    fn match_to_slot(&mut self, buf: &[u8], token: &[u8]) -> Option<usize> {
        let mid = u16::from_be_bytes(buf[2..4].try_into().unwrap());

        let token = u16::from_be_bytes(token[..].try_into().ok()?);

        let requests = self.shared.requests.borrow();
        for (slot, r) in requests.iter().enumerate() {
            if let RequestSent(details) = r {
                // FIXME check MID on piggybacked, as `details.mid == msgid &&`?
                if details.token == token {
                    return Some(slot);
                }
            }
        }
        None
    }

    /// Returns true if the response was expected
    async fn handle_response(&mut self, slot: usize) {
        let mut requests = self.shared.requests.borrow_mut();
        let RequestSent(details) = requests[slot] else {
            unreachable!();
        };
        requests[slot] = ResponsePending(details);
        drop(requests);

        self.shared.loop_to_app[slot].signal(());
        // Across here we should only have the slot number as a local variable
        self.shared.app_to_loop.wait().await;
        // FIXME: Actually, there could be any reason, plus we'd really need to clear: So
        // we should loop, clearing the signal, while our buffer is not BufferFilled, and
        // then at any rate signal again so that we don't lose signals.

        // Not checking whether this was left Empty or RequestSent -- either way, we
        // processed it, and are done.
    }
}
