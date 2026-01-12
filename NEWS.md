# Changes in 0.1.0-alpha.5

* Breaking: Updated rand_core dependency from 0.6 to 0.9.
* Updated internal embassy-sync and heapless dependencies.
* Updated edition to 2024.
* Linter and CI fixes.

# Changes in 0.1.0-alpha.4

*This is a breaking change*

* Update embedded_nal_async from 0.7 to 0.8.
* CI fixes.

# Changes in 0.1.0-alpha.3

* Add defmt feature, derive defmt::Format on errors.

# Changes in 0.1.0-alpha.2

*This is a breaking change*

* Update to coap-message 0.3 and embedded-nal-async 0.7.

* Fix memory leak when the client application drops its state.

* Default token length limit is set to 8.

* Add tests.

* Various refactorings.
