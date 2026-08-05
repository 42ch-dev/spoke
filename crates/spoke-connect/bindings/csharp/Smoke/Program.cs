using SpokeConnectSmoke.Tests;

// Golden-parity smoke for the C# binding of the spoke-connect sync-core FFI
// facade. Every check asserts the C# surface reproduces the Rust golden
// vectors; the program exits 0 only when all checks pass. The checks cover
// the full exported surface: all 8 functions, all 3 objects (construction,
// method calls, disposal), both error enums, and the collection (`string[]`)
// + optional (`string?`) marshalling paths — all deterministic, no network
// or time dependence.
try
{
    GoldenParity.AssertDerivePeerId();
    Console.WriteLine($"derive_peer_id: PASS        # {GoldenParity.GoldenPeerId}");

    GoldenParity.AssertSignHelloSignature();
    Console.WriteLine("sign_hello signature: PASS  # golden signature bytes in signed envelope");

    GoldenParity.AssertVerifyHello();
    Console.WriteLine("verify_hello: PASS");

    GoldenParity.AssertTamperedHelloRejected();
    Console.WriteLine("tampered_hello: PASS        # rejected with CoreException.InvalidHelloSignature");

    GoldenParity.AssertProtocolVersion();
    Console.WriteLine($"protocol: {GoldenParity.ProtocolVersion}");

    GoldenParity.AssertAllowlistBehavior();
    Console.WriteLine("allowlist: PASS             # string[] marshalling; empty fails closed");

    GoldenParity.AssertDispatchGate();
    Console.WriteLine("dispatch gate: PASS         # spoke-baseline grants check; custom-op denied");

    GoldenParity.AssertRequiredCapability();
    Console.WriteLine("required_capability: PASS   # string? marshalling: Some / None");

    GoldenParity.AssertResponseCorrelation();
    Console.WriteLine("response correlation: PASS  # exact echo ok; mismatch -> CorrelationMismatch");

    GoldenParity.AssertNonceStoreLifecycle();
    Console.WriteLine("nonce_store: PASS           # first use ok; replay rejected; per-peer scope");

    GoldenParity.AssertSequenceObjectsLifecycle();
    Console.WriteLine("sequence objects: PASS      # outbound 0/1; inbound advance + replay mismatch");

#if SMOKE_HOST
    LoopbackSmoke.Run();
    Console.WriteLine("loopback RemoteAdapterFFI: PASS");
#endif

    Console.WriteLine();
    Console.WriteLine("GOLDEN PARITY: ALL PASS");
    return 0;
}
catch (Exception ex)
{
    Console.Error.WriteLine($"GOLDEN PARITY: FAIL — {ex.Message}");
    return 1;
}
