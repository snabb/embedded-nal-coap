> [!IMPORTANT]
> **This is a fork** of [chrysn/embedded-nal-coap](https://codeberg.org/chrysn/embedded-nal-coap)
> 0.1.0-alpha.6, with these bugs fixed on `master`:
>
> * **Eight-byte tokens are rejected.** `udp_read::parse` checked `tkl >= TKL_MAX`, so a
>   request with the longest token RFC 7252 §5.3.1 allows got an RST (CON) or no answer
>   (NON). It now accepts up to `TKL_MAX` inclusive.
>
> The fixes are not submitted upstream: they were written with AI assistance, and Codeberg
> does not accept AI-generated code. The version number is unchanged, so the fork can be
> used through `[patch.crates-io]` with a git `rev`.

# embedded-nal-coap ![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue) [![embedded-nal-coap on crates.io](https://img.shields.io/crates/v/embedded-nal-coap)](https://crates.io/crates/embedded-nal-coap) [![embedded-nal-coap on docs.rs](https://docs.rs/embedded-nal-coap/badge.svg)](https://docs.rs/embedded-nal-coap) [![Source Code Repository](https://img.shields.io/badge/Code-On%20Codeberg-blue?logo=Codeberg)](https://codeberg.org/chrysn/embedded-nal-coap)

A CoAP server and client implementation built on [embedded_nal_async][__link0].

### Usage and operation

An example of how to use this will be available as part of the [coap-message-demos][__link1] crate.

* Allocate a [CoAPShared][__link2] with a `CONCURRENT_REQUESTS` const of the number of outgoing requests
  that should be servicable simultaneously.

* [CoAPShared::split()][__link3] that into a client and a server part.

* Use the client’s [CoAPRuntimeClient::to()][__link4] method to create a [coap_request::Stack][__link5] that can
  be used to send CoAP requests and receive a response. (Multiple responses, eg. from
  observation, are planned but not implemented yet).
  
  The [coap-request-implementations][__link6] crate contains suitable (albeit immature) building blocks
  for constructing requests.

* Into the server’s [CoAPRunner::run()][__link7] method, pass an unconnected [UDP
  socket][__link8], a source of low-grade entropy (for
  retransmission jitter and that like) and a [CoAP server application][__link9].
  
  The [coap-handler-implementations][__link10] crate contains suitable building blocks for constructing
  such a server application (including some to combine handlers for individual resources into a
  handler that picks sub-handlers from the URI path).
  
  The future returned by the run function needs to be polled by an executor; note that it is
  not Send, and some executors need configuration to allow that.

### Caveats

* The server does not perform any amplification mitigation (and the handler can’t for lack of
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

* Messages are created with as little copying as [`embedded_nal_async`][__link11] permits. For writable messages,
  that means that they need to be written to in ascending CoAP option number. This is in
  accordance with the implemented [`coap_message::MinimalWritableMessage`][__link12] and
  [`coap_message::MutableWritableMessage`][__link13] traits.
  
  That restriction enables this crate to not only be `no_std`, but to not require `alloc`
  either.

### Choices

This implementation of CoAP chooses to go with a single task, thus only ever allocating a
single buffer as part of the task. There are certainly alternative choices to be made here,
which may either be implemented in a different crate or altered later (for example, if it turns
out that a more effective implementation uses different tasks for send and receive, but uses a
single buffer or an at-least-1-sized pool that gets locked by the tasks).


 [__cargo_doc2readme_dependencies_info]: ggGkYW0CYXSEG_W_Gn_kaocAGwCcVPfenh7eGy6gYLEwyIe4G6-xw_FwcbpjYXKEG2R7VyeTF1PlGwQQXbRi58ywG32uc44H21YZG63EgUDCVUYqYWSFgmxjb2FwX2hhbmRsZXJlMC4yLjGCbGNvYXBfbWVzc2FnZWUwLjMuNoJsY29hcF9yZXF1ZXN0bTAuMi4wLWFscGhhLjKDcWVtYmVkZGVkLW5hbC1jb2FwbTAuMS4wLWFscGhhLjZxZW1iZWRkZWRfbmFsX2NvYXCCcmVtYmVkZGVkX25hbF9hc3luY2UwLjkuMA
 [__link0]: https://crates.io/crates/embedded_nal_async/0.9.0
 [__link1]: https://crates.io/crates/coap-message-demos
 [__link10]: https://crates.io/crates/coap-handler-implementations
 [__link11]: https://crates.io/crates/embedded_nal_async/0.9.0
 [__link12]: https://docs.rs/coap_message/0.3.6/coap_message/?search=MinimalWritableMessage
 [__link13]: https://docs.rs/coap_message/0.3.6/coap_message/?search=MutableWritableMessage
 [__link2]: https://docs.rs/embedded-nal-coap/0.1.0-alpha.6/embedded_nal_coap/struct.CoAPShared.html
 [__link3]: CoAPShared::split()
 [__link4]: CoAPRuntimeClient::to()
 [__link5]: https://docs.rs/coap_request/0.2.0-alpha.2/coap_request/?search=Stack
 [__link6]: https://crates.io/crates/coap-request-implementations
 [__link7]: CoAPRunner::run()
 [__link8]: https://docs.rs/embedded_nal_async/0.9.0/embedded_nal_async/?search=UnconnectedUdp
 [__link9]: https://docs.rs/coap_handler/0.2.1/coap_handler/?search=Handler
