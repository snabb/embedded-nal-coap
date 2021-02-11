[![Build Status](https://gitlab.com/chrysn/embedded-nal-minimal-coapserver/badges/master/pipeline.svg)](https://gitlab.com/chrysn/embedded-nal-minimal-coapserver/commits/master)
![Maintenance](https://img.shields.io/badge/maintenance-experimental-blue.svg)

# embedded-nal-minimal-coapserver

A minimal CoAP server implementation built on [embedded_nal].

Usage and operation
-------------------

Until the project has matured further, see [the example] for usage. The general steps are:

[the example]: https://gitlab.com/chrysn/coap-message-demos/-/blob/master/examples/std_embedded_nal_minicoapserver.rs

* Get a network stack with a UDP server socket that implments [embedded_nal::UdpServer]

* Create a CoAP handler that implements [coap_handler::Handler]; the
  [coap_handler::implementations] module contains some building blocks
  (including some to combine handlers for individual resources into a handler that picks
  sub-handlers from the URI path).

* Whenever there is indication that a request might have come in, call [poll] with the stack,
  the socket and the handler. This will accept the data from the socket, decode the CoAP
  message, pass it to the handler, and send back the response.

  It returns successful if a message was processed (or something came in that could be
  ignored), propagates out errors from the socket, and returns [WouldBlock](embedded_nal::nb::Error::WouldBlock) if it
  turms out the socket was *not* ready.

  By applying the belowmentioned constraints and exercising some of the liberties designed into
  CoAP, the server does not need to hold any state of its own.

Caveats
-------

* The server does not perform any amplification mitigation (and the handler can't for lack of
  remote information); use this only in environments where this is acceptable (e.g. in closed
  networks).

* The server does not perform any message deduplication. All handler functions must therefore
  be idempotent.

* The response logic is implemented using [nb](embedded_nal::nb) and does not attempt to store responses for
  later invocations. If a request comes in and the response can't be sent right away, it is
  discarded.

* Being based on embedded-nal, it binds to the any-address but leaves the network stack to
  choose the sending address; this leads to subtle bugs when runnign on a system with multiple
  IP addresses.

* Messages are created with as little copying as [embedded_nal] permits. For writable messages,
  that means that they need to be written to in ascending CoAP option number. This is in
  accordance with the implemented [coap_message::MinimalWritableMessage] and
  [coap_message::MutableWritableMessage] traits.

  That restriction enables this crate to not only be `no_std`, but to not require `alloc`
  either.

Roadmap
-------

The goal of this server is to become a component that can be used easily to bring CoAP
connectivity to embedded devices at the bare minimum, while still being practically usable.

This means that the amplification mitigation topic will need to be addressed, and that security
backing must be catered for (probably by referring to an OSCORE/EDHOC mix-in).

Other than that, this implementation's plan is to stay simple and utilize the optimizations
CoAP offers, even if this means limiting the application (eg. to immediate responses, and to
idempotent handlers).

License: MIT OR Apache-2.0
