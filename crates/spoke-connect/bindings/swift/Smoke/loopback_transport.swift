// loopback_transport.swift — foreign-callback `Transport` that delegates to one
// end of an in-repo loopback pair (mirror of the Rust `LoopbackCallback` test
// helper). Shared by the smoke-host RemoteAdapter loopback, the multi-peer
// router smoke, and the tool-faces loopback smoke (which runs against the
// committed production binding, no smoke host needed).

import Foundation

final class LoopbackCallbackTransport: Transport {
    private let inner: LoopbackTransport

    init(inner: LoopbackTransport) {
        self.inner = inner
    }

    func send(envelope: Data) throws {
        try inner.send(envelope: envelope)
    }

    func recv() throws -> Data {
        try inner.recv()
    }

    func close() throws {
        try inner.close()
    }
}
