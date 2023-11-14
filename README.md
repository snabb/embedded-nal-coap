![Maintenance](https://img.shields.io/badge/maintenance-experimental-blue.svg)

# embedded-nal-coap

A CoAP server and client implementation built on [embedded_nal_async].

Usage and operation
-------------------

An example of how to use this will be available as part of the [coap-message-demos] crate.

* Open a UDP server socket that implements [embedded_nal_async::UnconnectedUdp]

* If you want to run a server,
  create a CoAP handler that implements [coap_handler::Handler]; the
  [coap-handler-implementations] module contains some building blocks
  (including some to combine handlers for individual resources into a handler that picks
  sub-handlers from the URI path).

* If you want to send requests,
  FIXME crate some kind of channel.

* Pick an async executor suitable for your platform, and run a FIXME run() function as a task,
  providing a UDP socket address and the inputs from above.

  By applying the belowmentioned constraints and exercising some of the liberties designed into
  CoAP, the server does not need to hold any state of its own.
  (FIXME revisit)

.. [coap-message-demos]: https://crates.io/crates/coap-message-demos
.. [coap-handler-implementations]: https://crates.io/crates/coap-handler-implementations

Caveats
-------

* The server does not perform any amplification mitigation (and the handler can't for lack of
  remote information); use this only in environments where this is acceptable (e.g. in closed
  networks).

  FIXME only provide the 3x buffer for responses / when the handler indicates that it needs
  more, 4.01 Echo? (The handler may be unhappy that it gets dropped; -handlers may need
  guidance on this)

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
