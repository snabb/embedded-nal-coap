# Changes in 0.3.0

* embedded-nal dependency changed from 0.2 to 0.6.

# Changes in 0.2.1

* Remove accidental std dependency

# Changes in 0.2.0
    
* Use the newer coap-handler and thus coap-message traits.

  Even though this does not change any local code, this constitutes a breaking
  change, as the server now only accepts the new traits.
