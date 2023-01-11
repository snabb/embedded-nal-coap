# Chagnes in 0.3.1

* A function `poll_with_response_handler` was added to allow the implementation
  of a CoAP client on top of this crate.
* Stray CON responses are responsed to with the RST the protocol requires.
* Options and payload processing moved into coap-message-utils, which is now
  used in its version 0.2.1.

# Changes in 0.3.0

* embedded-nal dependency changed from 0.2 to 0.6.

# Changes in 0.2.1

* Remove accidental std dependency

# Changes in 0.2.0
    
* Use the newer coap-handler and thus coap-message traits.

  Even though this does not change any local code, this constitutes a breaking
  change, as the server now only accepts the new traits.
