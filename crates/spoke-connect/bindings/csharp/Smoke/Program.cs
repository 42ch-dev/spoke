using SpokeConnectSmoke.Tests;

// Golden-parity smoke for the C# binding of the spoke-connect sync-core FFI
// facade. Every check asserts the C# surface reproduces the Rust golden
// vectors; the program exits 0 only when all checks pass.
try
{
    GoldenParity.AssertDerivePeerId();
    Console.WriteLine("derive_peer_id: PASS        # 12D3KooWJ1TsijH7H5F74hfAD5XishQz3sxrmAtVY37GtNd9CqYf");

    GoldenParity.AssertSignHelloSignature();
    Console.WriteLine("sign_hello signature: PASS  # golden signature bytes in signed envelope");

    GoldenParity.AssertVerifyHello();
    Console.WriteLine("verify_hello: PASS");

    GoldenParity.AssertTamperedHelloRejected();
    Console.WriteLine("tampered_hello: PASS        # rejected with CoreException.InvalidHelloSignature");

    GoldenParity.AssertProtocolVersion();
    Console.WriteLine($"protocol: {GoldenParity.ProtocolVersion}");

    Console.WriteLine();
    Console.WriteLine("GOLDEN PARITY: ALL PASS");
    return 0;
}
catch (Exception ex)
{
    Console.Error.WriteLine($"GOLDEN PARITY: FAIL — {ex.Message}");
    return 1;
}
