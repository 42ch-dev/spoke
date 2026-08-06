// Package spokeconnect is the integrator-facing Go binding for spoke-connect session core.
// Generated cgo internals live in generated/spoke_connect; this package re-exports the FFI surface.
package spokeconnect

import sc "github.com/42ch-dev/spoke/crates/spoke-connect/bindings/go/generated/spoke_connect"

type (
	InboundSequence          = sc.InboundSequence
	InboundSequenceInterface = sc.InboundSequenceInterface
	LoopbackTransport        = sc.LoopbackTransport
	LoopbackTransportInterface = sc.LoopbackTransportInterface
	LoopbackTransportPair    = sc.LoopbackTransportPair
	LoopbackTransportPairInterface = sc.LoopbackTransportPairInterface
	NonceStore               = sc.NonceStore
	NonceStoreInterface      = sc.NonceStoreInterface
	OutboundSequence         = sc.OutboundSequence
	OutboundSequenceInterface = sc.OutboundSequenceInterface
	RemoteAdapterFfi         = sc.RemoteAdapterFfi
	RemoteAdapterFfiInterface = sc.RemoteAdapterFfiInterface
	Transport                = sc.Transport

	CoreError                              = sc.CoreError
	CoreErrorInvalidHelloSignature         = sc.CoreErrorInvalidHelloSignature
	CoreErrorNonceReplay                   = sc.CoreErrorNonceReplay
	CoreErrorHandshakeFailed               = sc.CoreErrorHandshakeFailed
	CoreErrorInvalidNonce                  = sc.CoreErrorInvalidNonce
	CoreErrorCrypto                        = sc.CoreErrorCrypto
	CoreErrorJcs                           = sc.CoreErrorJcs
	CoreErrorTokenInvalid                  = sc.CoreErrorTokenInvalid
	CoreInvokeError                        = sc.CoreInvokeError
	CoreInvokeErrorSequenceExhausted       = sc.CoreInvokeErrorSequenceExhausted
	CoreInvokeErrorInboundSequenceMismatch = sc.CoreInvokeErrorInboundSequenceMismatch
	CoreInvokeErrorCorrelationMismatch     = sc.CoreInvokeErrorCorrelationMismatch
	FfiError                               = sc.FfiError
	FfiErrorDial                           = sc.FfiErrorDial
	FfiErrorRejected                       = sc.FfiErrorRejected
	TransportError                         = sc.TransportError
	TransportErrorClosed                   = sc.TransportErrorClosed
	TransportErrorIo                       = sc.TransportErrorIo
)

var (
	NewInboundSequence  = sc.NewInboundSequence
	NewNonceStore       = sc.NewNonceStore
	NewOutboundSequence = sc.NewOutboundSequence

	ErrCoreErrorInvalidHelloSignature         = sc.ErrCoreErrorInvalidHelloSignature
	ErrCoreErrorNonceReplay                   = sc.ErrCoreErrorNonceReplay
	ErrCoreErrorHandshakeFailed               = sc.ErrCoreErrorHandshakeFailed
	ErrCoreErrorInvalidNonce                  = sc.ErrCoreErrorInvalidNonce
	ErrCoreErrorCrypto                        = sc.ErrCoreErrorCrypto
	ErrCoreErrorJcs                           = sc.ErrCoreErrorJcs
	ErrCoreErrorTokenInvalid                  = sc.ErrCoreErrorTokenInvalid
	ErrCoreInvokeErrorSequenceExhausted       = sc.ErrCoreInvokeErrorSequenceExhausted
	ErrCoreInvokeErrorInboundSequenceMismatch = sc.ErrCoreInvokeErrorInboundSequenceMismatch
	ErrCoreInvokeErrorCorrelationMismatch     = sc.ErrCoreInvokeErrorCorrelationMismatch
	ErrFfiErrorDial                           = sc.ErrFfiErrorDial
	ErrFfiErrorRejected                       = sc.ErrFfiErrorRejected
	ErrTransportErrorClosed                   = sc.ErrTransportErrorClosed
	ErrTransportErrorIo                       = sc.ErrTransportErrorIo

	CheckResponseCorrelation      = sc.CheckResponseCorrelation
	ConnectRemoteAdapterFfi       = sc.ConnectRemoteAdapterFfi
	DerivePeerIdFromEd25519Pubkey = sc.DerivePeerIdFromEd25519Pubkey
	DispatchAllowed               = sc.DispatchAllowed
	IsAllowlisted                 = sc.IsAllowlisted
	NewLoopbackTransportPair     = sc.NewLoopbackTransportPair
	ProtocolVersion               = sc.ProtocolVersion
	RequiredCapability            = sc.RequiredCapability
	SignHelloEd25519              = sc.SignHelloEd25519
	VerifyHelloEd25519            = sc.VerifyHelloEd25519
)
