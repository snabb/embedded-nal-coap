![Maintenance](https://img.shields.io/badge/maintenance-experimental-blue.svg)

# embedded-nal-coap

A CoAP server and client implementation built on [embedded_nal_async].

Usage and operation
-------------------

An example of how to use this will be available as part of the [coap-message-demos] crate.

* Allocate a [CoAPShared] with a `CONCURRENT_REQUESTS` const of the number of outgoing requests
  that should be servicable simultaneously.

* [CoAPShared::split()] that into a client and a server part.

* Use the client's [CoAPRuntimeClient::to()] method to create a [coap_request::Stack] that can
  be used to send CoAP requests and receive a response. (Multiple responses, eg. from
  observation, are planned but not implemented yet).

  The [coap_request_implementations] crate contains suitable (albeit immature) building blocks
  for constructing requests.

* Into the server's [CoAPRunner::run()] method, pass an unconnected [UDP
  socket](embedded_nal_async::UnconnectedUdp), a source of low-grade entropy (for
  retransmission jitter and that like) and a [CoAP server application](coap_handler::Handler).

  The [coap_handler_implementations] crate contains suitable building blocks for constructing
  such a server application (including some to combine handlers for individual resources into a
  handler that picks sub-handlers from the URI path).

  The future returned by the run function needs to be polled by an executor; note that it is
  not Send, and some executors need configuration to allow that.


Caveats
-------

* The server does not perform any amplification mitigation (and the handler can't for lack of
  remote information); use this only in environments where this is acceptable (e.g. in closed
  networks).

  This will be mitigated in a future version.

  FIXME only provide the 3x buffer for responses / when the handler indicates that it needs
  more, 4.01 Echo? (The handler may be unhappy that it gets dropped; -handlers may need
  guidance on this)

* This server does not uphold NSTART and PROBING_RATE, the fundamental flow control parameters
  of CoAP that make it OK for generic Internet applications.

  This will be addressed in a future version.

  FIXME pass in a time source

* The server does not perform any message deduplication. All handler functions must therefore
  be idempotent.

* Messages are created with as little copying as [embedded_nal] permits. For writable messages,
  that means that they need to be written to in ascending CoAP option number. This is in
  accordance with the implemented [coap_message::MinimalWritableMessage] and
  [coap_message::MutableWritableMessage] traits.

  That restriction enables this crate to not only be `no_std`, but to not require `alloc`
  either.

Choices
-------

This implementation of CoAP chooses to go with a single task, thus only ever allocating a
single buffer as part of the task. There are certainly alternative choices to be made here,
which may either be implemented in a different crate or altered later (for example, if it turns
out that a more effective implementation uses different tasks for send and receive, but uses a
single buffer or an at-least-1-sized pool that gets locked by the tasks).

License: MIT OR Apache-2.0
