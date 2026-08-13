
package spoke_connect

// #include <spoke_connect.h>
import "C"

import (
	"bytes"
	"fmt"
	"io"
	"unsafe"
	"encoding/binary"
	"errors"
	"math"
	"runtime"
	"sync"
	"sync/atomic"
)



// This is needed, because as of go 1.24
// type RustBuffer C.RustBuffer cannot have methods,
// RustBuffer is treated as non-local type
type GoRustBuffer struct {
	inner C.RustBuffer
}

type RustBufferI interface {
	AsReader() *bytes.Reader
	Free()
	ToGoBytes() []byte
	Data() unsafe.Pointer
	Len() uint64
	Capacity() uint64
}

// C.RustBuffer fields exposed as an interface so they can be accessed in different Go packages.
// See https://github.com/golang/go/issues/13467
type ExternalCRustBuffer interface {
	Data() unsafe.Pointer
	Len() uint64
	Capacity() uint64
}

func RustBufferFromC(b C.RustBuffer) ExternalCRustBuffer {
	return GoRustBuffer {
		inner: b,
	}
}

func CFromRustBuffer(b ExternalCRustBuffer) C.RustBuffer {
	return C.RustBuffer {
		capacity: C.uint64_t(b.Capacity()),
		len: C.uint64_t(b.Len()),
		data: (*C.uchar)(b.Data()),
	}
}

func RustBufferFromExternal(b ExternalCRustBuffer) GoRustBuffer {
	return GoRustBuffer {
		inner: C.RustBuffer {
			capacity: C.uint64_t(b.Capacity()),
			len: C.uint64_t(b.Len()),
			data: (*C.uchar)(b.Data()),
		},
	}
}

func (cb GoRustBuffer) Capacity() uint64 {
	return uint64(cb.inner.capacity)
}

func (cb GoRustBuffer) Len() uint64 {
	return uint64(cb.inner.len)
}

func (cb GoRustBuffer) Data() unsafe.Pointer {
	return unsafe.Pointer(cb.inner.data)
}

func (cb GoRustBuffer) AsReader() *bytes.Reader {
	b := unsafe.Slice((*byte)(cb.inner.data), C.uint64_t(cb.inner.len))
	return bytes.NewReader(b)
}

func (cb GoRustBuffer) Free() {
	rustCall(func( status *C.RustCallStatus) bool {
		C.ffi_spoke_connect_rustbuffer_free(cb.inner, status)
		return false
	})
}

func (cb GoRustBuffer) ToGoBytes() []byte {
	return C.GoBytes(unsafe.Pointer(cb.inner.data), C.int(cb.inner.len))
}


func stringToRustBuffer(str string) C.RustBuffer {
	return bytesToRustBuffer([]byte(str))
}

func bytesToRustBuffer(b []byte) C.RustBuffer {
	if len(b) == 0 {
		return C.RustBuffer{}
	}
	// We can pass the pointer along here, as it is pinned
	// for the duration of this call
	foreign := C.ForeignBytes {
		len: C.int(len(b)),
		data: (*C.uchar)(unsafe.Pointer(&b[0])),
	}
	
	return rustCall(func( status *C.RustCallStatus) C.RustBuffer {
		return C.ffi_spoke_connect_rustbuffer_from_bytes(foreign, status)
	})
}


type BufLifter[GoType any] interface {
	Lift(value RustBufferI) GoType
}

type BufLowerer[GoType any] interface {
	Lower(value GoType) C.RustBuffer
}

type BufReader[GoType any] interface {
	Read(reader io.Reader) GoType
}

type BufWriter[GoType any] interface {
	Write(writer io.Writer, value GoType)
}

func LowerIntoRustBuffer[GoType any](bufWriter BufWriter[GoType], value GoType) C.RustBuffer {
	// This might be not the most efficient way but it does not require knowing allocation size
	// beforehand
	var buffer bytes.Buffer
	bufWriter.Write(&buffer, value)

	bytes, err := io.ReadAll(&buffer)
	if err != nil {
		panic(fmt.Errorf("reading written data: %w", err))
	}
	return bytesToRustBuffer(bytes)
}

func LiftFromRustBuffer[GoType any](bufReader BufReader[GoType], rbuf RustBufferI) GoType {
	defer rbuf.Free()
	reader := rbuf.AsReader()
	item := bufReader.Read(reader)
	if reader.Len() > 0 {
		// TODO: Remove this
		leftover, _ := io.ReadAll(reader)
		panic(fmt.Errorf("Junk remaining in buffer after lifting: %s", string(leftover)))
	}
	return item
}



func rustCallWithError[E any, U any](converter BufReader[E], callback func(*C.RustCallStatus) U) (U, E) {
	var status C.RustCallStatus
	returnValue := callback(&status)
	err := checkCallStatus(converter, status)
	return returnValue, err
}

func checkCallStatus[E any](converter BufReader[E], status C.RustCallStatus) E {
	switch status.code {
	case 0:
		var zero E
		return zero
	case 1:
		return LiftFromRustBuffer(converter, GoRustBuffer { inner: status.errorBuf })
	case 2:
		// when the rust code sees a panic, it tries to construct a rustBuffer
		// with the message.  but if that code panics, then it just sends back
		// an empty buffer.
		if status.errorBuf.len > 0 {
			panic(fmt.Errorf("%s", FfiConverterStringINSTANCE.Lift(GoRustBuffer { inner: status.errorBuf })))
		} else {
			panic(fmt.Errorf("Rust panicked while handling Rust panic"))
		}
	default:
		panic(fmt.Errorf("unknown status code: %d", status.code))
	}
}

func checkCallStatusUnknown(status C.RustCallStatus) error {
	switch status.code {
	case 0:
		return nil
	case 1:
		panic(fmt.Errorf("function not returning an error returned an error"))
	case 2:
		// when the rust code sees a panic, it tries to construct a C.RustBuffer
		// with the message.  but if that code panics, then it just sends back
		// an empty buffer.
		if status.errorBuf.len > 0 {
			panic(fmt.Errorf("%s", FfiConverterStringINSTANCE.Lift(GoRustBuffer {
				inner: status.errorBuf,
			})))
		} else {
			panic(fmt.Errorf("Rust panicked while handling Rust panic"))
		}
	default:
		return fmt.Errorf("unknown status code: %d", status.code)
	}
}

func rustCall[U any](callback func(*C.RustCallStatus) U) U {
	returnValue, err := rustCallWithError[error](nil, callback)
	if err != nil {
		panic(err)
	}
	return returnValue
}

type NativeError interface {
	AsError() error
}


func writeInt8(writer io.Writer, value int8) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeUint8(writer io.Writer, value uint8) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeInt16(writer io.Writer, value int16) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeUint16(writer io.Writer, value uint16) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeInt32(writer io.Writer, value int32) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeUint32(writer io.Writer, value uint32) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeInt64(writer io.Writer, value int64) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeUint64(writer io.Writer, value uint64) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeFloat32(writer io.Writer, value float32) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeFloat64(writer io.Writer, value float64) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}


func readInt8(reader io.Reader) int8 {
	var result int8
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readUint8(reader io.Reader) uint8 {
	var result uint8
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readInt16(reader io.Reader) int16 {
	var result int16
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readUint16(reader io.Reader) uint16 {
	var result uint16
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readInt32(reader io.Reader) int32 {
	var result int32
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readUint32(reader io.Reader) uint32 {
	var result uint32
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readInt64(reader io.Reader) int64 {
	var result int64
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readUint64(reader io.Reader) uint64 {
	var result uint64
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readFloat32(reader io.Reader) float32 {
	var result float32
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readFloat64(reader io.Reader) float64 {
	var result float64
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func init() {
        
        FfiConverterCallbackInterfaceTransportINSTANCE.register();
        uniffiCheckChecksums()
}


func uniffiCheckChecksums() {
	// Get the bindings contract version from our ComponentInterface
	bindingsContractVersion := 30
	// Get the scaffolding contract version by calling the into the dylib
	scaffoldingContractVersion := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint32_t {
		return C.ffi_spoke_connect_uniffi_contract_version()
	})
	if bindingsContractVersion != int(scaffoldingContractVersion) {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: UniFFI contract version mismatch")
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_func_check_response_correlation()
	})
	if checksum != 57062 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_func_check_response_correlation: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_func_derive_peer_id_from_ed25519_pubkey()
	})
	if checksum != 37906 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_func_derive_peer_id_from_ed25519_pubkey: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_func_dispatch_allowed()
	})
	if checksum != 25989 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_func_dispatch_allowed: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_func_is_allowlisted()
	})
	if checksum != 52933 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_func_is_allowlisted: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_func_protocol_version()
	})
	if checksum != 50454 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_func_protocol_version: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_func_required_capability()
	})
	if checksum != 8417 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_func_required_capability: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_func_sign_hello_ed25519()
	})
	if checksum != 37896 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_func_sign_hello_ed25519: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_func_verify_hello_ed25519()
	})
	if checksum != 15847 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_func_verify_hello_ed25519: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_func_loopback_transport_pair()
	})
	if checksum != 40597 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_func_loopback_transport_pair: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_func_new_multi_peer_router_ffi()
	})
	if checksum != 46664 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_func_new_multi_peer_router_ffi: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_func_connect_remote_adapter_ffi()
	})
	if checksum != 44915 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_func_connect_remote_adapter_ffi: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_inboundsequence_advance()
	})
	if checksum != 17976 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_inboundsequence_advance: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_noncestore_check_and_record()
	})
	if checksum != 41909 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_noncestore_check_and_record: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_outboundsequence_allocate()
	})
	if checksum != 57422 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_outboundsequence_allocate: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_loopbacktransport_close()
	})
	if checksum != 20355 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_loopbacktransport_close: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_loopbacktransport_recv()
	})
	if checksum != 39606 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_loopbacktransport_recv: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_loopbacktransport_send()
	})
	if checksum != 16974 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_loopbacktransport_send: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_loopbacktransportpair_client()
	})
	if checksum != 9696 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_loopbacktransportpair_client: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_loopbacktransportpair_server()
	})
	if checksum != 13605 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_loopbacktransportpair_server: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_multipeerrouterffi_get_host_capability_manifest()
	})
	if checksum != 59432 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_multipeerrouterffi_get_host_capability_manifest: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_multipeerrouterffi_get_knowledge_entry()
	})
	if checksum != 22722 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_multipeerrouterffi_get_knowledge_entry: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_multipeerrouterffi_get_relation()
	})
	if checksum != 18470 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_multipeerrouterffi_get_relation: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_multipeerrouterffi_list_knowledge_entries()
	})
	if checksum != 38484 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_multipeerrouterffi_list_knowledge_entries: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_multipeerrouterffi_list_peer_host_capability_manifests()
	})
	if checksum != 61680 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_multipeerrouterffi_list_peer_host_capability_manifests: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_multipeerrouterffi_list_peers()
	})
	if checksum != 20421 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_multipeerrouterffi_list_peers: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_multipeerrouterffi_list_rules()
	})
	if checksum != 33290 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_multipeerrouterffi_list_rules: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_multipeerrouterffi_list_timeline_events()
	})
	if checksum != 63418 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_multipeerrouterffi_list_timeline_events: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_multipeerrouterffi_put_findings()
	})
	if checksum != 49739 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_multipeerrouterffi_put_findings: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_multipeerrouterffi_put_knowledge_entry()
	})
	if checksum != 21998 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_multipeerrouterffi_put_knowledge_entry: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_multipeerrouterffi_put_relation()
	})
	if checksum != 128 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_multipeerrouterffi_put_relation: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_multipeerrouterffi_register_peer()
	})
	if checksum != 58386 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_multipeerrouterffi_register_peer: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_multipeerrouterffi_unregister_peer()
	})
	if checksum != 56292 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_multipeerrouterffi_unregister_peer: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_remoteadapterffi_close()
	})
	if checksum != 18719 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_remoteadapterffi_close: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_remoteadapterffi_get_host_capability_manifest()
	})
	if checksum != 41950 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_remoteadapterffi_get_host_capability_manifest: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_remoteadapterffi_get_knowledge_entry()
	})
	if checksum != 44466 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_remoteadapterffi_get_knowledge_entry: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_remoteadapterffi_get_relation()
	})
	if checksum != 47371 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_remoteadapterffi_get_relation: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_remoteadapterffi_list_knowledge_entries()
	})
	if checksum != 8982 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_remoteadapterffi_list_knowledge_entries: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_remoteadapterffi_list_peer_host_capability_manifests()
	})
	if checksum != 7630 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_remoteadapterffi_list_peer_host_capability_manifests: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_remoteadapterffi_list_rules()
	})
	if checksum != 57745 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_remoteadapterffi_list_rules: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_remoteadapterffi_list_timeline_events()
	})
	if checksum != 30715 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_remoteadapterffi_list_timeline_events: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_remoteadapterffi_put_findings()
	})
	if checksum != 8509 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_remoteadapterffi_put_findings: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_remoteadapterffi_put_knowledge_entry()
	})
	if checksum != 46465 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_remoteadapterffi_put_knowledge_entry: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_remoteadapterffi_put_relation()
	})
	if checksum != 40493 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_remoteadapterffi_put_relation: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_remoteadapterffi_remote_manifest()
	})
	if checksum != 12957 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_remoteadapterffi_remote_manifest: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_remoteadapterffi_remote_peer_id()
	})
	if checksum != 25676 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_remoteadapterffi_remote_peer_id: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_remoteadapterffi_session_id()
	})
	if checksum != 15819 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_remoteadapterffi_session_id: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_remoteadapterffi_state()
	})
	if checksum != 28869 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_remoteadapterffi_state: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_constructor_inboundsequence_new()
	})
	if checksum != 21926 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_constructor_inboundsequence_new: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_constructor_noncestore_new()
	})
	if checksum != 5575 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_constructor_noncestore_new: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_constructor_outboundsequence_new()
	})
	if checksum != 6716 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_constructor_outboundsequence_new: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_transport_send()
	})
	if checksum != 34348 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_transport_send: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_transport_recv()
	})
	if checksum != 49799 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_transport_recv: UniFFI API checksum mismatch")
	}
	}
	{
	checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
		return C.uniffi_spoke_connect_checksum_method_transport_close()
	})
	if checksum != 43413 {
		// If this happens try cleaning and rebuilding your project
		panic("spoke_connect: uniffi_spoke_connect_checksum_method_transport_close: UniFFI API checksum mismatch")
	}
	}
}



type FfiConverterUint64 struct{}

var FfiConverterUint64INSTANCE = FfiConverterUint64{}

func (FfiConverterUint64) Lower(value uint64) C.uint64_t {
	return C.uint64_t(value)
}

func (FfiConverterUint64) Write(writer io.Writer, value uint64) {
	writeUint64(writer, value)
}

func (FfiConverterUint64) Lift(value C.uint64_t) uint64 {
	return uint64(value)
}

func (FfiConverterUint64) Read(reader io.Reader) uint64 {
	return readUint64(reader)
}

type FfiDestroyerUint64 struct {}

func (FfiDestroyerUint64) Destroy(_ uint64) {}

type FfiConverterInt64 struct{}

var FfiConverterInt64INSTANCE = FfiConverterInt64{}

func (FfiConverterInt64) Lower(value int64) C.int64_t {
	return C.int64_t(value)
}

func (FfiConverterInt64) Write(writer io.Writer, value int64) {
	writeInt64(writer, value)
}

func (FfiConverterInt64) Lift(value C.int64_t) int64 {
	return int64(value)
}

func (FfiConverterInt64) Read(reader io.Reader) int64 {
	return readInt64(reader)
}

type FfiDestroyerInt64 struct {}

func (FfiDestroyerInt64) Destroy(_ int64) {}

type FfiConverterBool struct{}

var FfiConverterBoolINSTANCE = FfiConverterBool{}

func (FfiConverterBool) Lower(value bool) C.int8_t {
	if value {
		return C.int8_t(1)
	}
	return C.int8_t(0)
}

func (FfiConverterBool) Write(writer io.Writer, value bool) {
	if value {
		writeInt8(writer, 1)
	} else {
		writeInt8(writer, 0)
	}
}

func (FfiConverterBool) Lift(value C.int8_t) bool {
	return value != 0
}

func (FfiConverterBool) Read(reader io.Reader) bool {
	return readInt8(reader) != 0
}

type FfiDestroyerBool struct {}

func (FfiDestroyerBool) Destroy(_ bool) {}

type FfiConverterString struct{}

var FfiConverterStringINSTANCE = FfiConverterString{}

func (FfiConverterString) Lift(rb RustBufferI) string {
	defer rb.Free()
	reader := rb.AsReader()
	b, err := io.ReadAll(reader)
	if err != nil {
		panic(fmt.Errorf("reading reader: %w", err))
	}
	return string(b)
}

func (FfiConverterString) Read(reader io.Reader) string {
	length := readInt32(reader)
	buffer := make([]byte, length)
	read_length, err := reader.Read(buffer)
	if err != nil && err != io.EOF {
		panic(err)
	}
	if read_length != int(length) {
		panic(fmt.Errorf("bad read length when reading string, expected %d, read %d", length, read_length))
	}
	return string(buffer)
}

func (FfiConverterString) Lower(value string) C.RustBuffer {
	return stringToRustBuffer(value)
}

func (c FfiConverterString) LowerExternal(value string) ExternalCRustBuffer {
	return RustBufferFromC(stringToRustBuffer(value))
}

func (FfiConverterString) Write(writer io.Writer, value string) {
	if len(value) > math.MaxInt32 {
		panic("String is too large to fit into Int32")
	}

	writeInt32(writer, int32(len(value)))
	write_length, err := io.WriteString(writer, value)
	if err != nil {
		panic(err)
	}
	if write_length != len(value) {
		panic(fmt.Errorf("bad write length when writing string, expected %d, written %d", len(value), write_length))
	}
}

type FfiDestroyerString struct {}

func (FfiDestroyerString) Destroy(_ string) {}

type FfiConverterBytes struct{}

var FfiConverterBytesINSTANCE = FfiConverterBytes{}

func (c FfiConverterBytes) Lower(value []byte) C.RustBuffer {
	return LowerIntoRustBuffer[[]byte](c, value)
}

func (c FfiConverterBytes) LowerExternal(value []byte) ExternalCRustBuffer {
	return RustBufferFromC(c.Lower(value))
}

func (c FfiConverterBytes) Write(writer io.Writer, value []byte) {
	if len(value) > math.MaxInt32 {
		panic("[]byte is too large to fit into Int32")
	}

	writeInt32(writer, int32(len(value)))
	write_length, err := writer.Write(value)
	if err != nil {
		panic(err)
	}
	if write_length != len(value) {
		panic(fmt.Errorf("bad write length when writing []byte, expected %d, written %d", len(value), write_length))
	}
}

func (c FfiConverterBytes) Lift(rb RustBufferI) []byte {
	return LiftFromRustBuffer[[]byte](c, rb)
}

func (c FfiConverterBytes) Read(reader io.Reader) []byte {
	length := readInt32(reader)
	buffer := make([]byte, length)
	read_length, err := reader.Read(buffer)
	if err != nil && err != io.EOF {
		panic(err)
	}
	if read_length != int(length) {
		panic(fmt.Errorf("bad read length when reading []byte, expected %d, read %d", length, read_length))
	}
	return buffer
}

type FfiDestroyerBytes struct {}

func (FfiDestroyerBytes) Destroy(_ []byte) {}



// Below is an implementation of synchronization requirements outlined in the link.
// https://github.com/mozilla/uniffi-rs/blob/0dc031132d9493ca812c3af6e7dd60ad2ea95bf0/uniffi_bindgen/src/bindings/kotlin/templates/ObjectRuntime.kt#L31

type FfiObject struct {
	handle C.uint64_t
	callCounter atomic.Int64
	cloneFunction func(C.uint64_t, *C.RustCallStatus) C.uint64_t
	freeFunction func(C.uint64_t, *C.RustCallStatus)
	destroyed atomic.Bool
}

func newFfiObject(
	handle C.uint64_t,
	cloneFunction func(C.uint64_t, *C.RustCallStatus) C.uint64_t,
	freeFunction func(C.uint64_t, *C.RustCallStatus),
) FfiObject {
	return FfiObject {
		handle: handle,
		cloneFunction: cloneFunction,
		freeFunction: freeFunction,
	}
}

func (ffiObject *FfiObject)incrementPointer(debugName string) C.uint64_t {
	for {
		counter := ffiObject.callCounter.Load()
		if counter <= -1 {
			panic(fmt.Errorf("%v object has already been destroyed", debugName))
		}
		if counter == math.MaxInt64 {
			panic(fmt.Errorf("%v object call counter would overflow", debugName))
		}
		if ffiObject.callCounter.CompareAndSwap(counter, counter + 1) {
			break
		}
	}

	return rustCall(func(status *C.RustCallStatus) C.uint64_t {
		return ffiObject.cloneFunction(ffiObject.handle, status)
	})
}

func (ffiObject *FfiObject)decrementPointer() {
	if ffiObject.callCounter.Add(-1) == -1 {
		ffiObject.freeRustArcPtr()
	}
}

func (ffiObject *FfiObject)destroy() {
	if ffiObject.destroyed.CompareAndSwap(false, true) {
		if ffiObject.callCounter.Add(-1) == -1 {
			ffiObject.freeRustArcPtr()
		}
	}
}

func (ffiObject *FfiObject)freeRustArcPtr() {
	if ffiObject.handle == 0 {
		return
	}
	rustCall(func(status *C.RustCallStatus) int32 {
		ffiObject.freeFunction(ffiObject.handle, status)
		return 0
	})
}
// Inbound sequence expectation — thread-safe FFI wrapper over the core
// expectation, starting at 0.
type InboundSequenceInterface interface {
	// Accepts `sequence` iff it equals the next expected inbound sequence;
	// on acceptance the expectation advances by 1 and the new expectation
	// is returned. A replayed or out-of-order sequence yields
	// `InboundSequenceMismatch` and the expectation is left unchanged — the
	// caller must reject the invoke without dispatching it.
	Advance(sequence int64) (uint64, error)
}
// Inbound sequence expectation — thread-safe FFI wrapper over the core
// expectation, starting at 0.
type InboundSequence struct {
	ffiObject FfiObject
}
// Creates an expectation starting at 0 (the first accepted sequence).
func NewInboundSequence() *InboundSequence {
	return FfiConverterInboundSequenceINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint64_t {
		return C.uniffi_spoke_connect_fn_constructor_inboundsequence_new(_uniffiStatus)
	}))
}




// Accepts `sequence` iff it equals the next expected inbound sequence;
// on acceptance the expectation advances by 1 and the new expectation
// is returned. A replayed or out-of-order sequence yields
// `InboundSequenceMismatch` and the expectation is left unchanged — the
// caller must reject the invoke without dispatching it.
func (_self *InboundSequence) Advance(sequence int64) (uint64, error) {
	_pointer := _self.ffiObject.incrementPointer("*InboundSequence")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*CoreInvokeError](FfiConverterCoreInvokeError{},func(_uniffiStatus *C.RustCallStatus) C.uint64_t {
		return C.uniffi_spoke_connect_fn_method_inboundsequence_advance(
		_pointer,FfiConverterInt64INSTANCE.Lower(sequence),_uniffiStatus)
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue uint64
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterUint64INSTANCE.Lift(_uniffiRV), nil
		}
}
func (object *InboundSequence) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterInboundSequence struct {}

var FfiConverterInboundSequenceINSTANCE = FfiConverterInboundSequence{}


func (c FfiConverterInboundSequence) Lift(handle C.uint64_t) *InboundSequence {
	result := &InboundSequence {
		newFfiObject(
			handle,
			func(handle C.uint64_t, status *C.RustCallStatus) C.uint64_t {
				return C.uniffi_spoke_connect_fn_clone_inboundsequence(handle, status)
			},
			func(handle C.uint64_t, status *C.RustCallStatus) {
				C.uniffi_spoke_connect_fn_free_inboundsequence(handle, status)
			},
		),
	}
	runtime.SetFinalizer(result, (*InboundSequence).Destroy)
	return result
}

func (c FfiConverterInboundSequence) Read(reader io.Reader) *InboundSequence {
	return c.Lift(C.uint64_t(readUint64(reader)))
}

func (c FfiConverterInboundSequence) Lower(value *InboundSequence) C.uint64_t {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the handle will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked handle.
	handle := value.ffiObject.incrementPointer("*InboundSequence")
	defer value.ffiObject.decrementPointer()
	return handle
}

func (c FfiConverterInboundSequence) Write(writer io.Writer, value *InboundSequence) {
	writeUint64(writer, uint64(c.Lower(value)))
}

func LiftFromExternalInboundSequence(handle uint64) *InboundSequence {
	return FfiConverterInboundSequenceINSTANCE.Lift(C.uint64_t(handle))
}

func LowerToExternalInboundSequence(value *InboundSequence) uint64 {
	return uint64(FfiConverterInboundSequenceINSTANCE.Lower(value))
}

type FfiDestroyerInboundSequence struct {}

func (_ FfiDestroyerInboundSequence) Destroy(value *InboundSequence) {
		value.Destroy()
}



// One end of an in-memory loopback connection, exposed over FFI (AR-7)
// so a binding can exercise the callback `Transport` surface without a
// real network carrier. `send` delivers to the peer's `recv`; `close`
// closes the whole connection (both directions). Each method
// `block_on`s the shared runtime — the same synchronous block-on-async
// surface a binding uses (AR-1 / AR-6).
type LoopbackTransportInterface interface {
	// Close the whole connection (both directions). Idempotent.
	Close() error
	// Receive the next inbound envelope. Errors when the connection
	// closes.
	Recv() ([]byte, error)
	// Send one envelope; delivered to the peer end's `recv`.
	Send(envelope []byte) error
}
// One end of an in-memory loopback connection, exposed over FFI (AR-7)
// so a binding can exercise the callback `Transport` surface without a
// real network carrier. `send` delivers to the peer's `recv`; `close`
// closes the whole connection (both directions). Each method
// `block_on`s the shared runtime — the same synchronous block-on-async
// surface a binding uses (AR-1 / AR-6).
type LoopbackTransport struct {
	ffiObject FfiObject
}




// Close the whole connection (both directions). Idempotent.
func (_self *LoopbackTransport) Close() error {
	_pointer := _self.ffiObject.incrementPointer("*LoopbackTransport")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[*TransportError](FfiConverterTransportError{},func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_spoke_connect_fn_method_loopbacktransport_close(
		_pointer,_uniffiStatus)
		return false
	})
		return _uniffiErr.AsError()
}

// Receive the next inbound envelope. Errors when the connection
// closes.
func (_self *LoopbackTransport) Recv() ([]byte, error) {
	_pointer := _self.ffiObject.incrementPointer("*LoopbackTransport")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*TransportError](FfiConverterTransportError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_loopbacktransport_recv(
		_pointer,_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue []byte
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterBytesINSTANCE.Lift(_uniffiRV), nil
		}
}

// Send one envelope; delivered to the peer end's `recv`.
func (_self *LoopbackTransport) Send(envelope []byte) error {
	_pointer := _self.ffiObject.incrementPointer("*LoopbackTransport")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[*TransportError](FfiConverterTransportError{},func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_spoke_connect_fn_method_loopbacktransport_send(
		_pointer,FfiConverterBytesINSTANCE.Lower(envelope),_uniffiStatus)
		return false
	})
		return _uniffiErr.AsError()
}
func (object *LoopbackTransport) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterLoopbackTransport struct {}

var FfiConverterLoopbackTransportINSTANCE = FfiConverterLoopbackTransport{}


func (c FfiConverterLoopbackTransport) Lift(handle C.uint64_t) *LoopbackTransport {
	result := &LoopbackTransport {
		newFfiObject(
			handle,
			func(handle C.uint64_t, status *C.RustCallStatus) C.uint64_t {
				return C.uniffi_spoke_connect_fn_clone_loopbacktransport(handle, status)
			},
			func(handle C.uint64_t, status *C.RustCallStatus) {
				C.uniffi_spoke_connect_fn_free_loopbacktransport(handle, status)
			},
		),
	}
	runtime.SetFinalizer(result, (*LoopbackTransport).Destroy)
	return result
}

func (c FfiConverterLoopbackTransport) Read(reader io.Reader) *LoopbackTransport {
	return c.Lift(C.uint64_t(readUint64(reader)))
}

func (c FfiConverterLoopbackTransport) Lower(value *LoopbackTransport) C.uint64_t {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the handle will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked handle.
	handle := value.ffiObject.incrementPointer("*LoopbackTransport")
	defer value.ffiObject.decrementPointer()
	return handle
}

func (c FfiConverterLoopbackTransport) Write(writer io.Writer, value *LoopbackTransport) {
	writeUint64(writer, uint64(c.Lower(value)))
}

func LiftFromExternalLoopbackTransport(handle uint64) *LoopbackTransport {
	return FfiConverterLoopbackTransportINSTANCE.Lift(C.uint64_t(handle))
}

func LowerToExternalLoopbackTransport(value *LoopbackTransport) uint64 {
	return uint64(FfiConverterLoopbackTransportINSTANCE.Lower(value))
}

type FfiDestroyerLoopbackTransport struct {}

func (_ FfiDestroyerLoopbackTransport) Destroy(value *LoopbackTransport) {
		value.Destroy()
}



// Back-to-back loopback transport pair — `client` and `server` ends of
// the same in-memory connection (mirror of
// [`transport::loopback_transport_pair`]).
type LoopbackTransportPairInterface interface {
	// The client end of the connection.
	Client() *LoopbackTransport
	// The server end of the connection.
	Server() *LoopbackTransport
}
// Back-to-back loopback transport pair — `client` and `server` ends of
// the same in-memory connection (mirror of
// [`transport::loopback_transport_pair`]).
type LoopbackTransportPair struct {
	ffiObject FfiObject
}




// The client end of the connection.
func (_self *LoopbackTransportPair) Client() *LoopbackTransport {
	_pointer := _self.ffiObject.incrementPointer("*LoopbackTransportPair")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterLoopbackTransportINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint64_t {
		return C.uniffi_spoke_connect_fn_method_loopbacktransportpair_client(
		_pointer,_uniffiStatus)
	}))
}

// The server end of the connection.
func (_self *LoopbackTransportPair) Server() *LoopbackTransport {
	_pointer := _self.ffiObject.incrementPointer("*LoopbackTransportPair")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterLoopbackTransportINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint64_t {
		return C.uniffi_spoke_connect_fn_method_loopbacktransportpair_server(
		_pointer,_uniffiStatus)
	}))
}
func (object *LoopbackTransportPair) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterLoopbackTransportPair struct {}

var FfiConverterLoopbackTransportPairINSTANCE = FfiConverterLoopbackTransportPair{}


func (c FfiConverterLoopbackTransportPair) Lift(handle C.uint64_t) *LoopbackTransportPair {
	result := &LoopbackTransportPair {
		newFfiObject(
			handle,
			func(handle C.uint64_t, status *C.RustCallStatus) C.uint64_t {
				return C.uniffi_spoke_connect_fn_clone_loopbacktransportpair(handle, status)
			},
			func(handle C.uint64_t, status *C.RustCallStatus) {
				C.uniffi_spoke_connect_fn_free_loopbacktransportpair(handle, status)
			},
		),
	}
	runtime.SetFinalizer(result, (*LoopbackTransportPair).Destroy)
	return result
}

func (c FfiConverterLoopbackTransportPair) Read(reader io.Reader) *LoopbackTransportPair {
	return c.Lift(C.uint64_t(readUint64(reader)))
}

func (c FfiConverterLoopbackTransportPair) Lower(value *LoopbackTransportPair) C.uint64_t {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the handle will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked handle.
	handle := value.ffiObject.incrementPointer("*LoopbackTransportPair")
	defer value.ffiObject.decrementPointer()
	return handle
}

func (c FfiConverterLoopbackTransportPair) Write(writer io.Writer, value *LoopbackTransportPair) {
	writeUint64(writer, uint64(c.Lower(value)))
}

func LiftFromExternalLoopbackTransportPair(handle uint64) *LoopbackTransportPair {
	return FfiConverterLoopbackTransportPairINSTANCE.Lift(C.uint64_t(handle))
}

func LowerToExternalLoopbackTransportPair(value *LoopbackTransportPair) uint64 {
	return uint64(FfiConverterLoopbackTransportPairINSTANCE.Lower(value))
}

type FfiDestroyerLoopbackTransportPair struct {}

func (_ FfiDestroyerLoopbackTransportPair) Destroy(value *LoopbackTransportPair) {
		value.Destroy()
}



type MultiPeerRouterFfiInterface interface {
	GetHostCapabilityManifest() (string, error)
	GetKnowledgeEntry(entryId string) (string, error)
	GetRelation(relationId string) (string, error)
	ListKnowledgeEntries(scopeJson string) (string, error)
	ListPeerHostCapabilityManifests() (string, error)
	ListPeers() []string
	ListRules(ruleRefs []string) (string, error)
	ListTimelineEvents(scopeJson string) (string, error)
	PutFindings(findingsJson string) (string, error)
	PutKnowledgeEntry(entryJson string, expectedBaseRevision *uint64) (string, error)
	PutRelation(relationJson string, expectedBaseRevision *uint64) (string, error)
	RegisterPeer(adapter *RemoteAdapterFfi) (string, error)
	UnregisterPeer(peerId string) 
}
type MultiPeerRouterFfi struct {
	ffiObject FfiObject
}




func (_self *MultiPeerRouterFfi) GetHostCapabilityManifest() (string, error) {
	_pointer := _self.ffiObject.incrementPointer("*MultiPeerRouterFfi")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_multipeerrouterffi_get_host_capability_manifest(
		_pointer,_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

func (_self *MultiPeerRouterFfi) GetKnowledgeEntry(entryId string) (string, error) {
	_pointer := _self.ffiObject.incrementPointer("*MultiPeerRouterFfi")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_multipeerrouterffi_get_knowledge_entry(
		_pointer,FfiConverterStringINSTANCE.Lower(entryId),_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

func (_self *MultiPeerRouterFfi) GetRelation(relationId string) (string, error) {
	_pointer := _self.ffiObject.incrementPointer("*MultiPeerRouterFfi")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_multipeerrouterffi_get_relation(
		_pointer,FfiConverterStringINSTANCE.Lower(relationId),_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

func (_self *MultiPeerRouterFfi) ListKnowledgeEntries(scopeJson string) (string, error) {
	_pointer := _self.ffiObject.incrementPointer("*MultiPeerRouterFfi")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_multipeerrouterffi_list_knowledge_entries(
		_pointer,FfiConverterStringINSTANCE.Lower(scopeJson),_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

func (_self *MultiPeerRouterFfi) ListPeerHostCapabilityManifests() (string, error) {
	_pointer := _self.ffiObject.incrementPointer("*MultiPeerRouterFfi")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_multipeerrouterffi_list_peer_host_capability_manifests(
		_pointer,_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

func (_self *MultiPeerRouterFfi) ListPeers() []string {
	_pointer := _self.ffiObject.incrementPointer("*MultiPeerRouterFfi")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterSequenceStringINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_multipeerrouterffi_list_peers(
		_pointer,_uniffiStatus),
	}
	}))
}

func (_self *MultiPeerRouterFfi) ListRules(ruleRefs []string) (string, error) {
	_pointer := _self.ffiObject.incrementPointer("*MultiPeerRouterFfi")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_multipeerrouterffi_list_rules(
		_pointer,FfiConverterSequenceStringINSTANCE.Lower(ruleRefs),_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

func (_self *MultiPeerRouterFfi) ListTimelineEvents(scopeJson string) (string, error) {
	_pointer := _self.ffiObject.incrementPointer("*MultiPeerRouterFfi")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_multipeerrouterffi_list_timeline_events(
		_pointer,FfiConverterStringINSTANCE.Lower(scopeJson),_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

func (_self *MultiPeerRouterFfi) PutFindings(findingsJson string) (string, error) {
	_pointer := _self.ffiObject.incrementPointer("*MultiPeerRouterFfi")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_multipeerrouterffi_put_findings(
		_pointer,FfiConverterStringINSTANCE.Lower(findingsJson),_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

func (_self *MultiPeerRouterFfi) PutKnowledgeEntry(entryJson string, expectedBaseRevision *uint64) (string, error) {
	_pointer := _self.ffiObject.incrementPointer("*MultiPeerRouterFfi")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_multipeerrouterffi_put_knowledge_entry(
		_pointer,FfiConverterStringINSTANCE.Lower(entryJson), FfiConverterOptionalUint64INSTANCE.Lower(expectedBaseRevision),_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

func (_self *MultiPeerRouterFfi) PutRelation(relationJson string, expectedBaseRevision *uint64) (string, error) {
	_pointer := _self.ffiObject.incrementPointer("*MultiPeerRouterFfi")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_multipeerrouterffi_put_relation(
		_pointer,FfiConverterStringINSTANCE.Lower(relationJson), FfiConverterOptionalUint64INSTANCE.Lower(expectedBaseRevision),_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

func (_self *MultiPeerRouterFfi) RegisterPeer(adapter *RemoteAdapterFfi) (string, error) {
	_pointer := _self.ffiObject.incrementPointer("*MultiPeerRouterFfi")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_multipeerrouterffi_register_peer(
		_pointer,FfiConverterRemoteAdapterFfiINSTANCE.Lower(adapter),_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

func (_self *MultiPeerRouterFfi) UnregisterPeer(peerId string)  {
	_pointer := _self.ffiObject.incrementPointer("*MultiPeerRouterFfi")
	defer _self.ffiObject.decrementPointer()
	rustCall(func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_spoke_connect_fn_method_multipeerrouterffi_unregister_peer(
		_pointer,FfiConverterStringINSTANCE.Lower(peerId),_uniffiStatus)
		return false
	})
}
func (object *MultiPeerRouterFfi) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterMultiPeerRouterFfi struct {}

var FfiConverterMultiPeerRouterFfiINSTANCE = FfiConverterMultiPeerRouterFfi{}


func (c FfiConverterMultiPeerRouterFfi) Lift(handle C.uint64_t) *MultiPeerRouterFfi {
	result := &MultiPeerRouterFfi {
		newFfiObject(
			handle,
			func(handle C.uint64_t, status *C.RustCallStatus) C.uint64_t {
				return C.uniffi_spoke_connect_fn_clone_multipeerrouterffi(handle, status)
			},
			func(handle C.uint64_t, status *C.RustCallStatus) {
				C.uniffi_spoke_connect_fn_free_multipeerrouterffi(handle, status)
			},
		),
	}
	runtime.SetFinalizer(result, (*MultiPeerRouterFfi).Destroy)
	return result
}

func (c FfiConverterMultiPeerRouterFfi) Read(reader io.Reader) *MultiPeerRouterFfi {
	return c.Lift(C.uint64_t(readUint64(reader)))
}

func (c FfiConverterMultiPeerRouterFfi) Lower(value *MultiPeerRouterFfi) C.uint64_t {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the handle will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked handle.
	handle := value.ffiObject.incrementPointer("*MultiPeerRouterFfi")
	defer value.ffiObject.decrementPointer()
	return handle
}

func (c FfiConverterMultiPeerRouterFfi) Write(writer io.Writer, value *MultiPeerRouterFfi) {
	writeUint64(writer, uint64(c.Lower(value)))
}

func LiftFromExternalMultiPeerRouterFfi(handle uint64) *MultiPeerRouterFfi {
	return FfiConverterMultiPeerRouterFfiINSTANCE.Lift(C.uint64_t(handle))
}

func LowerToExternalMultiPeerRouterFfi(value *MultiPeerRouterFfi) uint64 {
	return uint64(FfiConverterMultiPeerRouterFfiINSTANCE.Lower(value))
}

type FfiDestroyerMultiPeerRouterFfi struct {}

func (_ FfiDestroyerMultiPeerRouterFfi) Destroy(value *MultiPeerRouterFfi) {
		value.Destroy()
}



// Single-use `(peer_id, nonce)` replay store — thread-safe FFI wrapper over
// the core store.
type NonceStoreInterface interface {
	// Records `(peer_id, nonce)` unless it was already accepted; returns
	// `false` on replay. Call only after the hello passed every earlier
	// gate (allowlist, signature) so a rejected hello is not burned.
	CheckAndRecord(peerId string, nonce string) bool
}
// Single-use `(peer_id, nonce)` replay store — thread-safe FFI wrapper over
// the core store.
type NonceStore struct {
	ffiObject FfiObject
}
// Creates an empty store.
func NewNonceStore() *NonceStore {
	return FfiConverterNonceStoreINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint64_t {
		return C.uniffi_spoke_connect_fn_constructor_noncestore_new(_uniffiStatus)
	}))
}




// Records `(peer_id, nonce)` unless it was already accepted; returns
// `false` on replay. Call only after the hello passed every earlier
// gate (allowlist, signature) so a rejected hello is not burned.
func (_self *NonceStore) CheckAndRecord(peerId string, nonce string) bool {
	_pointer := _self.ffiObject.incrementPointer("*NonceStore")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterBoolINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.int8_t {
		return C.uniffi_spoke_connect_fn_method_noncestore_check_and_record(
		_pointer,FfiConverterStringINSTANCE.Lower(peerId), FfiConverterStringINSTANCE.Lower(nonce),_uniffiStatus)
	}))
}
func (object *NonceStore) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterNonceStore struct {}

var FfiConverterNonceStoreINSTANCE = FfiConverterNonceStore{}


func (c FfiConverterNonceStore) Lift(handle C.uint64_t) *NonceStore {
	result := &NonceStore {
		newFfiObject(
			handle,
			func(handle C.uint64_t, status *C.RustCallStatus) C.uint64_t {
				return C.uniffi_spoke_connect_fn_clone_noncestore(handle, status)
			},
			func(handle C.uint64_t, status *C.RustCallStatus) {
				C.uniffi_spoke_connect_fn_free_noncestore(handle, status)
			},
		),
	}
	runtime.SetFinalizer(result, (*NonceStore).Destroy)
	return result
}

func (c FfiConverterNonceStore) Read(reader io.Reader) *NonceStore {
	return c.Lift(C.uint64_t(readUint64(reader)))
}

func (c FfiConverterNonceStore) Lower(value *NonceStore) C.uint64_t {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the handle will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked handle.
	handle := value.ffiObject.incrementPointer("*NonceStore")
	defer value.ffiObject.decrementPointer()
	return handle
}

func (c FfiConverterNonceStore) Write(writer io.Writer, value *NonceStore) {
	writeUint64(writer, uint64(c.Lower(value)))
}

func LiftFromExternalNonceStore(handle uint64) *NonceStore {
	return FfiConverterNonceStoreINSTANCE.Lift(C.uint64_t(handle))
}

func LowerToExternalNonceStore(value *NonceStore) uint64 {
	return uint64(FfiConverterNonceStoreINSTANCE.Lower(value))
}

type FfiDestroyerNonceStore struct {}

func (_ FfiDestroyerNonceStore) Destroy(value *NonceStore) {
		value.Destroy()
}



// Outbound sequence counter — thread-safe FFI wrapper over the core
// counter, starting at 0.
type OutboundSequenceInterface interface {
	// Assigns the next outbound sequence; on exhaustion (past the JSON-safe
	// wire maximum) `SequenceExhausted` is returned and the counter stays
	// exhausted — sequences never wrap. The caller must close the session.
	Allocate() (uint64, error)
}
// Outbound sequence counter — thread-safe FFI wrapper over the core
// counter, starting at 0.
type OutboundSequence struct {
	ffiObject FfiObject
}
// Creates a counter starting at 0 (the first allocate returns 0).
func NewOutboundSequence() *OutboundSequence {
	return FfiConverterOutboundSequenceINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint64_t {
		return C.uniffi_spoke_connect_fn_constructor_outboundsequence_new(_uniffiStatus)
	}))
}




// Assigns the next outbound sequence; on exhaustion (past the JSON-safe
// wire maximum) `SequenceExhausted` is returned and the counter stays
// exhausted — sequences never wrap. The caller must close the session.
func (_self *OutboundSequence) Allocate() (uint64, error) {
	_pointer := _self.ffiObject.incrementPointer("*OutboundSequence")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*CoreInvokeError](FfiConverterCoreInvokeError{},func(_uniffiStatus *C.RustCallStatus) C.uint64_t {
		return C.uniffi_spoke_connect_fn_method_outboundsequence_allocate(
		_pointer,_uniffiStatus)
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue uint64
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterUint64INSTANCE.Lift(_uniffiRV), nil
		}
}
func (object *OutboundSequence) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterOutboundSequence struct {}

var FfiConverterOutboundSequenceINSTANCE = FfiConverterOutboundSequence{}


func (c FfiConverterOutboundSequence) Lift(handle C.uint64_t) *OutboundSequence {
	result := &OutboundSequence {
		newFfiObject(
			handle,
			func(handle C.uint64_t, status *C.RustCallStatus) C.uint64_t {
				return C.uniffi_spoke_connect_fn_clone_outboundsequence(handle, status)
			},
			func(handle C.uint64_t, status *C.RustCallStatus) {
				C.uniffi_spoke_connect_fn_free_outboundsequence(handle, status)
			},
		),
	}
	runtime.SetFinalizer(result, (*OutboundSequence).Destroy)
	return result
}

func (c FfiConverterOutboundSequence) Read(reader io.Reader) *OutboundSequence {
	return c.Lift(C.uint64_t(readUint64(reader)))
}

func (c FfiConverterOutboundSequence) Lower(value *OutboundSequence) C.uint64_t {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the handle will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked handle.
	handle := value.ffiObject.incrementPointer("*OutboundSequence")
	defer value.ffiObject.decrementPointer()
	return handle
}

func (c FfiConverterOutboundSequence) Write(writer io.Writer, value *OutboundSequence) {
	writeUint64(writer, uint64(c.Lower(value)))
}

func LiftFromExternalOutboundSequence(handle uint64) *OutboundSequence {
	return FfiConverterOutboundSequenceINSTANCE.Lift(C.uint64_t(handle))
}

func LowerToExternalOutboundSequence(value *OutboundSequence) uint64 {
	return uint64(FfiConverterOutboundSequenceINSTANCE.Lower(value))
}

type FfiDestroyerOutboundSequence struct {}

func (_ FfiDestroyerOutboundSequence) Destroy(value *OutboundSequence) {
		value.Destroy()
}



type RemoteAdapterFfiInterface interface {
	Close() 
	GetHostCapabilityManifest() (string, error)
	GetKnowledgeEntry(entryId string) (string, error)
	GetRelation(relationId string) (string, error)
	ListKnowledgeEntries(scopeJson string) (string, error)
	ListPeerHostCapabilityManifests() (string, error)
	ListRules(ruleRefs []string) (string, error)
	ListTimelineEvents(scopeJson string) (string, error)
	PutFindings(findingsJson string) (string, error)
	PutKnowledgeEntry(entryJson string, expectedBaseRevision *uint64) (string, error)
	PutRelation(relationJson string, expectedBaseRevision *uint64) (string, error)
	RemoteManifest() *string
	RemotePeerId() *string
	SessionId() *string
	State() string
}
type RemoteAdapterFfi struct {
	ffiObject FfiObject
}




func (_self *RemoteAdapterFfi) Close()  {
	_pointer := _self.ffiObject.incrementPointer("*RemoteAdapterFfi")
	defer _self.ffiObject.decrementPointer()
	rustCall(func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_spoke_connect_fn_method_remoteadapterffi_close(
		_pointer,_uniffiStatus)
		return false
	})
}

func (_self *RemoteAdapterFfi) GetHostCapabilityManifest() (string, error) {
	_pointer := _self.ffiObject.incrementPointer("*RemoteAdapterFfi")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_remoteadapterffi_get_host_capability_manifest(
		_pointer,_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

func (_self *RemoteAdapterFfi) GetKnowledgeEntry(entryId string) (string, error) {
	_pointer := _self.ffiObject.incrementPointer("*RemoteAdapterFfi")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_remoteadapterffi_get_knowledge_entry(
		_pointer,FfiConverterStringINSTANCE.Lower(entryId),_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

func (_self *RemoteAdapterFfi) GetRelation(relationId string) (string, error) {
	_pointer := _self.ffiObject.incrementPointer("*RemoteAdapterFfi")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_remoteadapterffi_get_relation(
		_pointer,FfiConverterStringINSTANCE.Lower(relationId),_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

func (_self *RemoteAdapterFfi) ListKnowledgeEntries(scopeJson string) (string, error) {
	_pointer := _self.ffiObject.incrementPointer("*RemoteAdapterFfi")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_remoteadapterffi_list_knowledge_entries(
		_pointer,FfiConverterStringINSTANCE.Lower(scopeJson),_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

func (_self *RemoteAdapterFfi) ListPeerHostCapabilityManifests() (string, error) {
	_pointer := _self.ffiObject.incrementPointer("*RemoteAdapterFfi")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_remoteadapterffi_list_peer_host_capability_manifests(
		_pointer,_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

func (_self *RemoteAdapterFfi) ListRules(ruleRefs []string) (string, error) {
	_pointer := _self.ffiObject.incrementPointer("*RemoteAdapterFfi")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_remoteadapterffi_list_rules(
		_pointer,FfiConverterSequenceStringINSTANCE.Lower(ruleRefs),_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

func (_self *RemoteAdapterFfi) ListTimelineEvents(scopeJson string) (string, error) {
	_pointer := _self.ffiObject.incrementPointer("*RemoteAdapterFfi")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_remoteadapterffi_list_timeline_events(
		_pointer,FfiConverterStringINSTANCE.Lower(scopeJson),_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

func (_self *RemoteAdapterFfi) PutFindings(findingsJson string) (string, error) {
	_pointer := _self.ffiObject.incrementPointer("*RemoteAdapterFfi")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_remoteadapterffi_put_findings(
		_pointer,FfiConverterStringINSTANCE.Lower(findingsJson),_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

func (_self *RemoteAdapterFfi) PutKnowledgeEntry(entryJson string, expectedBaseRevision *uint64) (string, error) {
	_pointer := _self.ffiObject.incrementPointer("*RemoteAdapterFfi")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_remoteadapterffi_put_knowledge_entry(
		_pointer,FfiConverterStringINSTANCE.Lower(entryJson), FfiConverterOptionalUint64INSTANCE.Lower(expectedBaseRevision),_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

func (_self *RemoteAdapterFfi) PutRelation(relationJson string, expectedBaseRevision *uint64) (string, error) {
	_pointer := _self.ffiObject.incrementPointer("*RemoteAdapterFfi")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_remoteadapterffi_put_relation(
		_pointer,FfiConverterStringINSTANCE.Lower(relationJson), FfiConverterOptionalUint64INSTANCE.Lower(expectedBaseRevision),_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

func (_self *RemoteAdapterFfi) RemoteManifest() *string {
	_pointer := _self.ffiObject.incrementPointer("*RemoteAdapterFfi")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterOptionalStringINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_remoteadapterffi_remote_manifest(
		_pointer,_uniffiStatus),
	}
	}))
}

func (_self *RemoteAdapterFfi) RemotePeerId() *string {
	_pointer := _self.ffiObject.incrementPointer("*RemoteAdapterFfi")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterOptionalStringINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_remoteadapterffi_remote_peer_id(
		_pointer,_uniffiStatus),
	}
	}))
}

func (_self *RemoteAdapterFfi) SessionId() *string {
	_pointer := _self.ffiObject.incrementPointer("*RemoteAdapterFfi")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterOptionalStringINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_remoteadapterffi_session_id(
		_pointer,_uniffiStatus),
	}
	}))
}

func (_self *RemoteAdapterFfi) State() string {
	_pointer := _self.ffiObject.incrementPointer("*RemoteAdapterFfi")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterStringINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_method_remoteadapterffi_state(
		_pointer,_uniffiStatus),
	}
	}))
}
func (object *RemoteAdapterFfi) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterRemoteAdapterFfi struct {}

var FfiConverterRemoteAdapterFfiINSTANCE = FfiConverterRemoteAdapterFfi{}


func (c FfiConverterRemoteAdapterFfi) Lift(handle C.uint64_t) *RemoteAdapterFfi {
	result := &RemoteAdapterFfi {
		newFfiObject(
			handle,
			func(handle C.uint64_t, status *C.RustCallStatus) C.uint64_t {
				return C.uniffi_spoke_connect_fn_clone_remoteadapterffi(handle, status)
			},
			func(handle C.uint64_t, status *C.RustCallStatus) {
				C.uniffi_spoke_connect_fn_free_remoteadapterffi(handle, status)
			},
		),
	}
	runtime.SetFinalizer(result, (*RemoteAdapterFfi).Destroy)
	return result
}

func (c FfiConverterRemoteAdapterFfi) Read(reader io.Reader) *RemoteAdapterFfi {
	return c.Lift(C.uint64_t(readUint64(reader)))
}

func (c FfiConverterRemoteAdapterFfi) Lower(value *RemoteAdapterFfi) C.uint64_t {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the handle will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked handle.
	handle := value.ffiObject.incrementPointer("*RemoteAdapterFfi")
	defer value.ffiObject.decrementPointer()
	return handle
}

func (c FfiConverterRemoteAdapterFfi) Write(writer io.Writer, value *RemoteAdapterFfi) {
	writeUint64(writer, uint64(c.Lower(value)))
}

func LiftFromExternalRemoteAdapterFfi(handle uint64) *RemoteAdapterFfi {
	return FfiConverterRemoteAdapterFfiINSTANCE.Lift(C.uint64_t(handle))
}

func LowerToExternalRemoteAdapterFfi(value *RemoteAdapterFfi) uint64 {
	return uint64(FfiConverterRemoteAdapterFfiINSTANCE.Lower(value))
}

type FfiDestroyerRemoteAdapterFfi struct {}

func (_ FfiDestroyerRemoteAdapterFfi) Destroy(value *RemoteAdapterFfi) {
		value.Destroy()
}


// FFI-facing mirror of [`crate::core::CoreError`] (hello-gate / identity
// failures). Mapped 1:1 in [`From<CoreErrorImpl>`].
type CoreError struct {
	err error
}

// Convenience method to turn *CoreError into error
// Avoiding treating nil pointer as non nil error interface
func (err *CoreError) AsError() error {
	if err == nil {
		return nil
	} else {
		return err
	}
}

func (err CoreError) Error() string {
	return fmt.Sprintf("CoreError: %s", err.err.Error())
}

func (err CoreError) Unwrap() error {
	return err.err
}

// Err* are used for checking error type with `errors.Is`
var ErrCoreErrorInvalidHelloSignature = fmt.Errorf("CoreErrorInvalidHelloSignature")
var ErrCoreErrorNonceReplay = fmt.Errorf("CoreErrorNonceReplay")
var ErrCoreErrorHandshakeFailed = fmt.Errorf("CoreErrorHandshakeFailed")
var ErrCoreErrorInvalidNonce = fmt.Errorf("CoreErrorInvalidNonce")
var ErrCoreErrorCrypto = fmt.Errorf("CoreErrorCrypto")
var ErrCoreErrorJcs = fmt.Errorf("CoreErrorJcs")
var ErrCoreErrorTokenInvalid = fmt.Errorf("CoreErrorTokenInvalid")
var ErrCoreErrorProtocolVersionMismatch = fmt.Errorf("CoreErrorProtocolVersionMismatch")

// Variant structs
// The hello signature did not verify against the peer's public key (or
// the signature is not valid base64url / not 64 bytes).
type CoreErrorInvalidHelloSignature struct {
}
// The hello signature did not verify against the peer's public key (or
// the signature is not valid base64url / not 64 bytes).
func NewCoreErrorInvalidHelloSignature(
) *CoreError {
	return &CoreError { err: &CoreErrorInvalidHelloSignature {} }
}

func (e CoreErrorInvalidHelloSignature) destroy() {
}


func (err CoreErrorInvalidHelloSignature) Error() string {
	return fmt.Sprint("InvalidHelloSignature",
		
	)
}

func (self CoreErrorInvalidHelloSignature) Is(target error) bool {
	return target == ErrCoreErrorInvalidHelloSignature
}
// The `(peer_id, nonce)` pair was already accepted.
type CoreErrorNonceReplay struct {
}
// The `(peer_id, nonce)` pair was already accepted.
func NewCoreErrorNonceReplay(
) *CoreError {
	return &CoreError { err: &CoreErrorNonceReplay {} }
}

func (e CoreErrorNonceReplay) destroy() {
}


func (err CoreErrorNonceReplay) Error() string {
	return fmt.Sprint("NonceReplay",
		
	)
}

func (self CoreErrorNonceReplay) Is(target error) bool {
	return target == ErrCoreErrorNonceReplay
}
// Handshake-level failure (peer id binding, dial binding, …).
type CoreErrorHandshakeFailed struct {
	Reason string
}
// Handshake-level failure (peer id binding, dial binding, …).
func NewCoreErrorHandshakeFailed(
	reason string,
) *CoreError {
	return &CoreError { err: &CoreErrorHandshakeFailed {
			Reason: reason,} }
}

func (e CoreErrorHandshakeFailed) destroy() {
		FfiDestroyerString{}.Destroy(e.Reason)
}


func (err CoreErrorHandshakeFailed) Error() string {
	return fmt.Sprint("HandshakeFailed",
		": ",
		
		"Reason=",
		err.Reason,
	)
}

func (self CoreErrorHandshakeFailed) Is(target error) bool {
	return target == ErrCoreErrorHandshakeFailed
}
// The hello nonce does not satisfy the wire constraints (minLength 16).
type CoreErrorInvalidNonce struct {
	Message string
}
// The hello nonce does not satisfy the wire constraints (minLength 16).
func NewCoreErrorInvalidNonce(
	message string,
) *CoreError {
	return &CoreError { err: &CoreErrorInvalidNonce {
			Message: message,} }
}

func (e CoreErrorInvalidNonce) destroy() {
		FfiDestroyerString{}.Destroy(e.Message)
}


func (err CoreErrorInvalidNonce) Error() string {
	return fmt.Sprint("InvalidNonce",
		": ",
		
		"Message=",
		err.Message,
	)
}

func (self CoreErrorInvalidNonce) Is(target error) bool {
	return target == ErrCoreErrorInvalidNonce
}
// Cryptography-level failure (invalid key bytes, base64 decoding, …).
type CoreErrorCrypto struct {
	Message string
}
// Cryptography-level failure (invalid key bytes, base64 decoding, …).
func NewCoreErrorCrypto(
	message string,
) *CoreError {
	return &CoreError { err: &CoreErrorCrypto {
			Message: message,} }
}

func (e CoreErrorCrypto) destroy() {
		FfiDestroyerString{}.Destroy(e.Message)
}


func (err CoreErrorCrypto) Error() string {
	return fmt.Sprint("Crypto",
		": ",
		
		"Message=",
		err.Message,
	)
}

func (self CoreErrorCrypto) Is(target error) bool {
	return target == ErrCoreErrorCrypto
}
// RFC 8785 JCS canonicalization / serialization of the signed object
// failed.
type CoreErrorJcs struct {
	Message string
}
// RFC 8785 JCS canonicalization / serialization of the signed object
// failed.
func NewCoreErrorJcs(
	message string,
) *CoreError {
	return &CoreError { err: &CoreErrorJcs {
			Message: message,} }
}

func (e CoreErrorJcs) destroy() {
		FfiDestroyerString{}.Destroy(e.Message)
}


func (err CoreErrorJcs) Error() string {
	return fmt.Sprint("Jcs",
		": ",
		
		"Message=",
		err.Message,
	)
}

func (self CoreErrorJcs) Is(target error) bool {
	return target == ErrCoreErrorJcs
}
// A capability-token proof failed validation (malformed shape, bad
// signature, untrusted issuer, subject/audience/expiry mismatch, or
// claim-rule violation).
type CoreErrorTokenInvalid struct {
	Message string
}
// A capability-token proof failed validation (malformed shape, bad
// signature, untrusted issuer, subject/audience/expiry mismatch, or
// claim-rule violation).
func NewCoreErrorTokenInvalid(
	message string,
) *CoreError {
	return &CoreError { err: &CoreErrorTokenInvalid {
			Message: message,} }
}

func (e CoreErrorTokenInvalid) destroy() {
		FfiDestroyerString{}.Destroy(e.Message)
}


func (err CoreErrorTokenInvalid) Error() string {
	return fmt.Sprint("TokenInvalid",
		": ",
		
		"Message=",
		err.Message,
	)
}

func (self CoreErrorTokenInvalid) Is(target error) bool {
	return target == ErrCoreErrorTokenInvalid
}
// The hello's `protocol_version` does not match the core protocol
// version — a mixed-version peer or a downgrade attempt. Dedicated
// classification for version negotiation failure, distinct from other
// handshake faults (spec §Error mapping: `details.kind =
// protocol_version_mismatch`). Appended after `TokenInvalid` so the
// pre-existing variants keep their binding ordinals.
type CoreErrorProtocolVersionMismatch struct {
	Reason string
}
// The hello's `protocol_version` does not match the core protocol
// version — a mixed-version peer or a downgrade attempt. Dedicated
// classification for version negotiation failure, distinct from other
// handshake faults (spec §Error mapping: `details.kind =
// protocol_version_mismatch`). Appended after `TokenInvalid` so the
// pre-existing variants keep their binding ordinals.
func NewCoreErrorProtocolVersionMismatch(
	reason string,
) *CoreError {
	return &CoreError { err: &CoreErrorProtocolVersionMismatch {
			Reason: reason,} }
}

func (e CoreErrorProtocolVersionMismatch) destroy() {
		FfiDestroyerString{}.Destroy(e.Reason)
}


func (err CoreErrorProtocolVersionMismatch) Error() string {
	return fmt.Sprint("ProtocolVersionMismatch",
		": ",
		
		"Reason=",
		err.Reason,
	)
}

func (self CoreErrorProtocolVersionMismatch) Is(target error) bool {
	return target == ErrCoreErrorProtocolVersionMismatch
}

type FfiConverterCoreError struct{}

var FfiConverterCoreErrorINSTANCE = FfiConverterCoreError{}

func (c FfiConverterCoreError) Lift(eb RustBufferI) *CoreError {
	return LiftFromRustBuffer[*CoreError](c, eb)
}

func (c FfiConverterCoreError) Lower(value *CoreError) C.RustBuffer {
	return LowerIntoRustBuffer[*CoreError](c, value)
}

func (c FfiConverterCoreError) LowerExternal(value *CoreError) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[*CoreError](c, value))
}

func (c FfiConverterCoreError) Read(reader io.Reader) *CoreError {
	errorID := readUint32(reader)

	switch errorID {
	case 1:
		return &CoreError{ &CoreErrorInvalidHelloSignature{
		}}
	case 2:
		return &CoreError{ &CoreErrorNonceReplay{
		}}
	case 3:
		return &CoreError{ &CoreErrorHandshakeFailed{
			Reason: FfiConverterStringINSTANCE.Read(reader),
		}}
	case 4:
		return &CoreError{ &CoreErrorInvalidNonce{
			Message: FfiConverterStringINSTANCE.Read(reader),
		}}
	case 5:
		return &CoreError{ &CoreErrorCrypto{
			Message: FfiConverterStringINSTANCE.Read(reader),
		}}
	case 6:
		return &CoreError{ &CoreErrorJcs{
			Message: FfiConverterStringINSTANCE.Read(reader),
		}}
	case 7:
		return &CoreError{ &CoreErrorTokenInvalid{
			Message: FfiConverterStringINSTANCE.Read(reader),
		}}
	case 8:
		return &CoreError{ &CoreErrorProtocolVersionMismatch{
			Reason: FfiConverterStringINSTANCE.Read(reader),
		}}
	default:
		panic(fmt.Sprintf("Unknown error code %d in FfiConverterCoreError.Read()", errorID))
	}
}

func (c FfiConverterCoreError) Write(writer io.Writer, value *CoreError) {
	switch variantValue := value.err.(type) {
		case *CoreErrorInvalidHelloSignature:
			writeInt32(writer, 1)
		case *CoreErrorNonceReplay:
			writeInt32(writer, 2)
		case *CoreErrorHandshakeFailed:
			writeInt32(writer, 3)
			FfiConverterStringINSTANCE.Write(writer, variantValue.Reason)
		case *CoreErrorInvalidNonce:
			writeInt32(writer, 4)
			FfiConverterStringINSTANCE.Write(writer, variantValue.Message)
		case *CoreErrorCrypto:
			writeInt32(writer, 5)
			FfiConverterStringINSTANCE.Write(writer, variantValue.Message)
		case *CoreErrorJcs:
			writeInt32(writer, 6)
			FfiConverterStringINSTANCE.Write(writer, variantValue.Message)
		case *CoreErrorTokenInvalid:
			writeInt32(writer, 7)
			FfiConverterStringINSTANCE.Write(writer, variantValue.Message)
		case *CoreErrorProtocolVersionMismatch:
			writeInt32(writer, 8)
			FfiConverterStringINSTANCE.Write(writer, variantValue.Reason)
		default:
			_ = variantValue
			panic(fmt.Sprintf("invalid error value `%v` in FfiConverterCoreError.Write", value))
	}
}

type FfiDestroyerCoreError struct {}

func (_ FfiDestroyerCoreError) Destroy(value *CoreError) {
	switch variantValue := value.err.(type) {
		case CoreErrorInvalidHelloSignature:
			variantValue.destroy()
		case CoreErrorNonceReplay:
			variantValue.destroy()
		case CoreErrorHandshakeFailed:
			variantValue.destroy()
		case CoreErrorInvalidNonce:
			variantValue.destroy()
		case CoreErrorCrypto:
			variantValue.destroy()
		case CoreErrorJcs:
			variantValue.destroy()
		case CoreErrorTokenInvalid:
			variantValue.destroy()
		case CoreErrorProtocolVersionMismatch:
			variantValue.destroy()
		default:
			_ = variantValue
			panic(fmt.Sprintf("invalid error value `%v` in FfiDestroyerCoreError.Destroy", value))
	}
}

// FFI-facing mirror of [`crate::core::CoreInvokeError`] (invoke-path
// sequence / correlation failures). Mapped 1:1 in
// [`From<CoreInvokeErrorImpl>`].
type CoreInvokeError struct {
	err error
}

// Convenience method to turn *CoreInvokeError into error
// Avoiding treating nil pointer as non nil error interface
func (err *CoreInvokeError) AsError() error {
	if err == nil {
		return nil
	} else {
		return err
	}
}

func (err CoreInvokeError) Error() string {
	return fmt.Sprintf("CoreInvokeError: %s", err.err.Error())
}

func (err CoreInvokeError) Unwrap() error {
	return err.err
}

// Err* are used for checking error type with `errors.Is`
var ErrCoreInvokeErrorSequenceExhausted = fmt.Errorf("CoreInvokeErrorSequenceExhausted")
var ErrCoreInvokeErrorInboundSequenceMismatch = fmt.Errorf("CoreInvokeErrorInboundSequenceMismatch")
var ErrCoreInvokeErrorCorrelationMismatch = fmt.Errorf("CoreInvokeErrorCorrelationMismatch")

// Variant structs
// The session's outbound sequence space (2⁵³−1) is exhausted; the
// session must be closed and reopened — sequences never wrap.
type CoreInvokeErrorSequenceExhausted struct {
}
// The session's outbound sequence space (2⁵³−1) is exhausted; the
// session must be closed and reopened — sequences never wrap.
func NewCoreInvokeErrorSequenceExhausted(
) *CoreInvokeError {
	return &CoreInvokeError { err: &CoreInvokeErrorSequenceExhausted {} }
}

func (e CoreInvokeErrorSequenceExhausted) destroy() {
}


func (err CoreInvokeErrorSequenceExhausted) Error() string {
	return fmt.Sprint("SequenceExhausted",
		
	)
}

func (self CoreInvokeErrorSequenceExhausted) Is(target error) bool {
	return target == ErrCoreInvokeErrorSequenceExhausted
}
// An inbound invoke `sequence` is not the next expected one (replay or
// out-of-order); the invoke must not be dispatched.
type CoreInvokeErrorInboundSequenceMismatch struct {
	Expected uint64
	Actual int64
}
// An inbound invoke `sequence` is not the next expected one (replay or
// out-of-order); the invoke must not be dispatched.
func NewCoreInvokeErrorInboundSequenceMismatch(
	expected uint64,
	actual int64,
) *CoreInvokeError {
	return &CoreInvokeError { err: &CoreInvokeErrorInboundSequenceMismatch {
			Expected: expected,
			Actual: actual,} }
}

func (e CoreInvokeErrorInboundSequenceMismatch) destroy() {
		FfiDestroyerUint64{}.Destroy(e.Expected)
		FfiDestroyerInt64{}.Destroy(e.Actual)
}


func (err CoreInvokeErrorInboundSequenceMismatch) Error() string {
	return fmt.Sprint("InboundSequenceMismatch",
		": ",
		
		"Expected=",
		err.Expected,
		", ",
		"Actual=",
		err.Actual,
	)
}

func (self CoreInvokeErrorInboundSequenceMismatch) Is(target error) bool {
	return target == ErrCoreInvokeErrorInboundSequenceMismatch
}
// A response did not echo the request's `session_id` / `sequence` /
// `request_id`.
type CoreInvokeErrorCorrelationMismatch struct {
}
// A response did not echo the request's `session_id` / `sequence` /
// `request_id`.
func NewCoreInvokeErrorCorrelationMismatch(
) *CoreInvokeError {
	return &CoreInvokeError { err: &CoreInvokeErrorCorrelationMismatch {} }
}

func (e CoreInvokeErrorCorrelationMismatch) destroy() {
}


func (err CoreInvokeErrorCorrelationMismatch) Error() string {
	return fmt.Sprint("CorrelationMismatch",
		
	)
}

func (self CoreInvokeErrorCorrelationMismatch) Is(target error) bool {
	return target == ErrCoreInvokeErrorCorrelationMismatch
}

type FfiConverterCoreInvokeError struct{}

var FfiConverterCoreInvokeErrorINSTANCE = FfiConverterCoreInvokeError{}

func (c FfiConverterCoreInvokeError) Lift(eb RustBufferI) *CoreInvokeError {
	return LiftFromRustBuffer[*CoreInvokeError](c, eb)
}

func (c FfiConverterCoreInvokeError) Lower(value *CoreInvokeError) C.RustBuffer {
	return LowerIntoRustBuffer[*CoreInvokeError](c, value)
}

func (c FfiConverterCoreInvokeError) LowerExternal(value *CoreInvokeError) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[*CoreInvokeError](c, value))
}

func (c FfiConverterCoreInvokeError) Read(reader io.Reader) *CoreInvokeError {
	errorID := readUint32(reader)

	switch errorID {
	case 1:
		return &CoreInvokeError{ &CoreInvokeErrorSequenceExhausted{
		}}
	case 2:
		return &CoreInvokeError{ &CoreInvokeErrorInboundSequenceMismatch{
			Expected: FfiConverterUint64INSTANCE.Read(reader),
			Actual: FfiConverterInt64INSTANCE.Read(reader),
		}}
	case 3:
		return &CoreInvokeError{ &CoreInvokeErrorCorrelationMismatch{
		}}
	default:
		panic(fmt.Sprintf("Unknown error code %d in FfiConverterCoreInvokeError.Read()", errorID))
	}
}

func (c FfiConverterCoreInvokeError) Write(writer io.Writer, value *CoreInvokeError) {
	switch variantValue := value.err.(type) {
		case *CoreInvokeErrorSequenceExhausted:
			writeInt32(writer, 1)
		case *CoreInvokeErrorInboundSequenceMismatch:
			writeInt32(writer, 2)
			FfiConverterUint64INSTANCE.Write(writer, variantValue.Expected)
			FfiConverterInt64INSTANCE.Write(writer, variantValue.Actual)
		case *CoreInvokeErrorCorrelationMismatch:
			writeInt32(writer, 3)
		default:
			_ = variantValue
			panic(fmt.Sprintf("invalid error value `%v` in FfiConverterCoreInvokeError.Write", value))
	}
}

type FfiDestroyerCoreInvokeError struct {}

func (_ FfiDestroyerCoreInvokeError) Destroy(value *CoreInvokeError) {
	switch variantValue := value.err.(type) {
		case CoreInvokeErrorSequenceExhausted:
			variantValue.destroy()
		case CoreInvokeErrorInboundSequenceMismatch:
			variantValue.destroy()
		case CoreInvokeErrorCorrelationMismatch:
			variantValue.destroy()
		default:
			_ = variantValue
			panic(fmt.Sprintf("invalid error value `%v` in FfiDestroyerCoreInvokeError.Destroy", value))
	}
}

// FFI error surface — 1:1 with frozen-contract D7 (AR-5).
//
// - [`FfiError::Dial`] — constructor / dial failures before an adapter
// exists (`config` / `handshake` / `timeout`).
// - [`FfiError::Rejected`] — invoke-path `SpokeResult::Reject` passthrough:
// application codes preserved; `INTERNAL_ERROR` rows carry `kind`;
// dispatch deny and unknown wire codes carry `wire_code`.
type FfiError struct {
	err error
}

// Convenience method to turn *FfiError into error
// Avoiding treating nil pointer as non nil error interface
func (err *FfiError) AsError() error {
	if err == nil {
		return nil
	} else {
		return err
	}
}

func (err FfiError) Error() string {
	return fmt.Sprintf("FfiError: %s", err.err.Error())
}

func (err FfiError) Unwrap() error {
	return err.err
}

// Err* are used for checking error type with `errors.Is`
var ErrFfiErrorDial = fmt.Errorf("FfiErrorDial")
var ErrFfiErrorRejected = fmt.Errorf("FfiErrorRejected")

// Variant structs
type FfiErrorDial struct {
	Kind string
	Message string
}
func NewFfiErrorDial(
	kind string,
	message string,
) *FfiError {
	return &FfiError { err: &FfiErrorDial {
			Kind: kind,
			Message: message,} }
}

func (e FfiErrorDial) destroy() {
		FfiDestroyerString{}.Destroy(e.Kind)
		FfiDestroyerString{}.Destroy(e.Message)
}


func (err FfiErrorDial) Error() string {
	return fmt.Sprint("Dial",
		": ",
		
		"Kind=",
		err.Kind,
		", ",
		"Message=",
		err.Message,
	)
}

func (self FfiErrorDial) Is(target error) bool {
	return target == ErrFfiErrorDial
}
type FfiErrorRejected struct {
	Code string
	Message string
	Kind *string
	WireCode *string
}
func NewFfiErrorRejected(
	code string,
	message string,
	kind *string,
	wireCode *string,
) *FfiError {
	return &FfiError { err: &FfiErrorRejected {
			Code: code,
			Message: message,
			Kind: kind,
			WireCode: wireCode,} }
}

func (e FfiErrorRejected) destroy() {
		FfiDestroyerString{}.Destroy(e.Code)
		FfiDestroyerString{}.Destroy(e.Message)
		FfiDestroyerOptionalString{}.Destroy(e.Kind)
		FfiDestroyerOptionalString{}.Destroy(e.WireCode)
}


func (err FfiErrorRejected) Error() string {
	return fmt.Sprint("Rejected",
		": ",
		
		"Code=",
		err.Code,
		", ",
		"Message=",
		err.Message,
		", ",
		"Kind=",
		err.Kind,
		", ",
		"WireCode=",
		err.WireCode,
	)
}

func (self FfiErrorRejected) Is(target error) bool {
	return target == ErrFfiErrorRejected
}

type FfiConverterFfiError struct{}

var FfiConverterFfiErrorINSTANCE = FfiConverterFfiError{}

func (c FfiConverterFfiError) Lift(eb RustBufferI) *FfiError {
	return LiftFromRustBuffer[*FfiError](c, eb)
}

func (c FfiConverterFfiError) Lower(value *FfiError) C.RustBuffer {
	return LowerIntoRustBuffer[*FfiError](c, value)
}

func (c FfiConverterFfiError) LowerExternal(value *FfiError) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[*FfiError](c, value))
}

func (c FfiConverterFfiError) Read(reader io.Reader) *FfiError {
	errorID := readUint32(reader)

	switch errorID {
	case 1:
		return &FfiError{ &FfiErrorDial{
			Kind: FfiConverterStringINSTANCE.Read(reader),
			Message: FfiConverterStringINSTANCE.Read(reader),
		}}
	case 2:
		return &FfiError{ &FfiErrorRejected{
			Code: FfiConverterStringINSTANCE.Read(reader),
			Message: FfiConverterStringINSTANCE.Read(reader),
			Kind: FfiConverterOptionalStringINSTANCE.Read(reader),
			WireCode: FfiConverterOptionalStringINSTANCE.Read(reader),
		}}
	default:
		panic(fmt.Sprintf("Unknown error code %d in FfiConverterFfiError.Read()", errorID))
	}
}

func (c FfiConverterFfiError) Write(writer io.Writer, value *FfiError) {
	switch variantValue := value.err.(type) {
		case *FfiErrorDial:
			writeInt32(writer, 1)
			FfiConverterStringINSTANCE.Write(writer, variantValue.Kind)
			FfiConverterStringINSTANCE.Write(writer, variantValue.Message)
		case *FfiErrorRejected:
			writeInt32(writer, 2)
			FfiConverterStringINSTANCE.Write(writer, variantValue.Code)
			FfiConverterStringINSTANCE.Write(writer, variantValue.Message)
			FfiConverterOptionalStringINSTANCE.Write(writer, variantValue.Kind)
			FfiConverterOptionalStringINSTANCE.Write(writer, variantValue.WireCode)
		default:
			_ = variantValue
			panic(fmt.Sprintf("invalid error value `%v` in FfiConverterFfiError.Write", value))
	}
}

type FfiDestroyerFfiError struct {}

func (_ FfiDestroyerFfiError) Destroy(value *FfiError) {
	switch variantValue := value.err.(type) {
		case FfiErrorDial:
			variantValue.destroy()
		case FfiErrorRejected:
			variantValue.destroy()
		default:
			_ = variantValue
			panic(fmt.Sprintf("invalid error value `%v` in FfiDestroyerFfiError.Destroy", value))
	}
}

// FFI-facing mirror of [`transport::TransportError`] — the
// callback `Transport`'s own error vocabulary. 1:1 with the remote
// error; the bridge maps both directions.
type TransportError struct {
	err error
}

// Convenience method to turn *TransportError into error
// Avoiding treating nil pointer as non nil error interface
func (err *TransportError) AsError() error {
	if err == nil {
		return nil
	} else {
		return err
	}
}

func (err TransportError) Error() string {
	return fmt.Sprintf("TransportError: %s", err.err.Error())
}

func (err TransportError) Unwrap() error {
	return err.err
}

// Err* are used for checking error type with `errors.Is`
var ErrTransportErrorClosed = fmt.Errorf("TransportErrorClosed")
var ErrTransportErrorIo = fmt.Errorf("TransportErrorIo")

// Variant structs
// The transport is closed. A pending `recv` must fail fast on
// connection loss so the adapter can fail its in-flight invokes.
type TransportErrorClosed struct {
}
// The transport is closed. A pending `recv` must fail fast on
// connection loss so the adapter can fail its in-flight invokes.
func NewTransportErrorClosed(
) *TransportError {
	return &TransportError { err: &TransportErrorClosed {} }
}

func (e TransportErrorClosed) destroy() {
}


func (err TransportErrorClosed) Error() string {
	return fmt.Sprint("Closed",
		
	)
}

func (self TransportErrorClosed) Is(target error) bool {
	return target == ErrTransportErrorClosed
}
// Transport-level I/O failure.
type TransportErrorIo struct {
	Field0 string
}
// Transport-level I/O failure.
func NewTransportErrorIo(
	var0 string,
) *TransportError {
	return &TransportError { err: &TransportErrorIo {
			Field0: var0,} }
}

func (e TransportErrorIo) destroy() {
		FfiDestroyerString{}.Destroy(e.Field0)
}


func (err TransportErrorIo) Error() string {
	return fmt.Sprint("Io",
		": ",
		
		"Field0=",
		err.Field0,
	)
}

func (self TransportErrorIo) Is(target error) bool {
	return target == ErrTransportErrorIo
}

type FfiConverterTransportError struct{}

var FfiConverterTransportErrorINSTANCE = FfiConverterTransportError{}

func (c FfiConverterTransportError) Lift(eb RustBufferI) *TransportError {
	return LiftFromRustBuffer[*TransportError](c, eb)
}

func (c FfiConverterTransportError) Lower(value *TransportError) C.RustBuffer {
	return LowerIntoRustBuffer[*TransportError](c, value)
}

func (c FfiConverterTransportError) LowerExternal(value *TransportError) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[*TransportError](c, value))
}

func (c FfiConverterTransportError) Read(reader io.Reader) *TransportError {
	errorID := readUint32(reader)

	switch errorID {
	case 1:
		return &TransportError{ &TransportErrorClosed{
		}}
	case 2:
		return &TransportError{ &TransportErrorIo{
			Field0: FfiConverterStringINSTANCE.Read(reader),
		}}
	default:
		panic(fmt.Sprintf("Unknown error code %d in FfiConverterTransportError.Read()", errorID))
	}
}

func (c FfiConverterTransportError) Write(writer io.Writer, value *TransportError) {
	switch variantValue := value.err.(type) {
		case *TransportErrorClosed:
			writeInt32(writer, 1)
		case *TransportErrorIo:
			writeInt32(writer, 2)
			FfiConverterStringINSTANCE.Write(writer, variantValue.Field0)
		default:
			_ = variantValue
			panic(fmt.Sprintf("invalid error value `%v` in FfiConverterTransportError.Write", value))
	}
}

type FfiDestroyerTransportError struct {}

func (_ FfiDestroyerTransportError) Destroy(value *TransportError) {
	switch variantValue := value.err.(type) {
		case TransportErrorClosed:
			variantValue.destroy()
		case TransportErrorIo:
			variantValue.destroy()
		default:
			_ = variantValue
			panic(fmt.Sprintf("invalid error value `%v` in FfiDestroyerTransportError.Destroy", value))
	}
}


// Message-oriented transport implemented by the foreign binding.
//
// Mirrors the async [`transport::Transport`] seam 1:1 over the
// FFI boundary (frozen contract §2.1): `send` accepts exactly one
// envelope's bytes, `recv` returns the next inbound envelope and fails
// fast on close, `close` is idempotent resource release.
type Transport interface {
	
	// Send one envelope. Resolves when the transport has accepted the
	// bytes.
	Send(envelope []byte) error
	
	// Receive the next inbound envelope. Errors when the transport
	// closes.
	Recv() ([]byte, error)
	
	// Release resources. Idempotent.
	Close() error
	
}


type FfiConverterCallbackInterfaceTransport struct {
	handleMap *concurrentHandleMap[Transport]
}

var FfiConverterCallbackInterfaceTransportINSTANCE = FfiConverterCallbackInterfaceTransport {
	handleMap: newConcurrentHandleMap[Transport](),
}

func (c FfiConverterCallbackInterfaceTransport) Lift(handle uint64) Transport {
	val, ok := c.handleMap.tryGet(handle)
	if !ok {
		panic(fmt.Errorf("no callback in handle map: %d", handle))
	}
	return val
}

func (c FfiConverterCallbackInterfaceTransport) Read(reader io.Reader) Transport {
	return c.Lift(readUint64(reader))
}

func (c FfiConverterCallbackInterfaceTransport) Lower(value Transport) C.uint64_t {
	return C.uint64_t(c.handleMap.insert(value))
}

func (c FfiConverterCallbackInterfaceTransport) Write(writer io.Writer, value Transport) {
	writeUint64(writer, uint64(c.Lower(value)))
}

func LiftFromExternalCallbackInterfaceTransport(handle uint64) Transport {
	return FfiConverterCallbackInterfaceTransportINSTANCE.Lift(handle)
}

func LowerToExternalCallbackInterfaceTransport(value Transport) uint64 {
	return uint64(FfiConverterCallbackInterfaceTransportINSTANCE.Lower(value))
}

type FfiDestroyerCallbackInterfaceTransport struct {}

func (FfiDestroyerCallbackInterfaceTransport) Destroy(value Transport) {}

type uniffiCallbackResult C.int8_t

const (
	uniffiIdxCallbackFree               uniffiCallbackResult = 0
	uniffiCallbackResultSuccess         uniffiCallbackResult = 0
	uniffiCallbackResultError           uniffiCallbackResult = 1
	uniffiCallbackUnexpectedResultError uniffiCallbackResult = 2
	uniffiCallbackCancelled             uniffiCallbackResult = 3
)


type concurrentHandleMap[T any] struct {
	handles       map[uint64]T
	currentHandle uint64
	lock          sync.RWMutex
}

func newConcurrentHandleMap[T any]() *concurrentHandleMap[T] {
	return &concurrentHandleMap[T]{
		handles:  map[uint64]T{},
		currentHandle: 1,
	}
}

func (cm *concurrentHandleMap[T]) insert(obj T) uint64 {
	cm.lock.Lock()
	defer cm.lock.Unlock()

	handle := cm.currentHandle
	cm.currentHandle = cm.currentHandle + 2
	cm.handles[handle] = obj
	return handle
}

func (cm *concurrentHandleMap[T]) remove(handle uint64) {
	cm.lock.Lock()
	defer cm.lock.Unlock()

	delete(cm.handles, handle)
}

func (cm *concurrentHandleMap[T]) tryGet(handle uint64) (T, bool) {
	cm.lock.RLock()
	defer cm.lock.RUnlock()

	val, ok := cm.handles[handle]
	return val, ok
}

//export spoke_connect_ffi_foreign_transport_cgo_dispatchCallbackInterfaceTransportMethod0
func spoke_connect_ffi_foreign_transport_cgo_dispatchCallbackInterfaceTransportMethod0(uniffiHandle C.uint64_t,envelope C.RustBuffer,uniffiOutReturn *C.void,callStatus *C.RustCallStatus,) {
	handle := uint64(uniffiHandle)
	uniffiObj, ok := FfiConverterCallbackInterfaceTransportINSTANCE.handleMap.tryGet(handle)
	if !ok {
		panic(fmt.Errorf("no callback in handle map: %d", handle))
	}
	
	

	 err :=
    uniffiObj.Send(
        FfiConverterBytesINSTANCE.Lift(GoRustBuffer {
		inner: envelope,
	}),
    )
	
    
	if err != nil {
		var actualError *TransportError
		if errors.As(err, &actualError) {
			*callStatus = C.RustCallStatus {
				code: C.int8_t(uniffiCallbackResultError),
				errorBuf: FfiConverterTransportErrorINSTANCE.Lower(actualError),
			}
		} else {
			*callStatus = C.RustCallStatus {
				code: C.int8_t(uniffiCallbackUnexpectedResultError),
			}
		}
		return
	}


	
}



//export spoke_connect_ffi_foreign_transport_cgo_dispatchCallbackInterfaceTransportMethod1
func spoke_connect_ffi_foreign_transport_cgo_dispatchCallbackInterfaceTransportMethod1(uniffiHandle C.uint64_t,uniffiOutReturn *C.RustBuffer,callStatus *C.RustCallStatus,) {
	handle := uint64(uniffiHandle)
	uniffiObj, ok := FfiConverterCallbackInterfaceTransportINSTANCE.handleMap.tryGet(handle)
	if !ok {
		panic(fmt.Errorf("no callback in handle map: %d", handle))
	}
	
	

	 res, err :=
    uniffiObj.Recv(
    )
	
    
	if err != nil {
		var actualError *TransportError
		if errors.As(err, &actualError) {
			*callStatus = C.RustCallStatus {
				code: C.int8_t(uniffiCallbackResultError),
				errorBuf: FfiConverterTransportErrorINSTANCE.Lower(actualError),
			}
		} else {
			*callStatus = C.RustCallStatus {
				code: C.int8_t(uniffiCallbackUnexpectedResultError),
			}
		}
		return
	}


	*uniffiOutReturn = FfiConverterBytesINSTANCE.Lower(res)
}



//export spoke_connect_ffi_foreign_transport_cgo_dispatchCallbackInterfaceTransportMethod2
func spoke_connect_ffi_foreign_transport_cgo_dispatchCallbackInterfaceTransportMethod2(uniffiHandle C.uint64_t,uniffiOutReturn *C.void,callStatus *C.RustCallStatus,) {
	handle := uint64(uniffiHandle)
	uniffiObj, ok := FfiConverterCallbackInterfaceTransportINSTANCE.handleMap.tryGet(handle)
	if !ok {
		panic(fmt.Errorf("no callback in handle map: %d", handle))
	}
	
	

	 err :=
    uniffiObj.Close(
    )
	
    
	if err != nil {
		var actualError *TransportError
		if errors.As(err, &actualError) {
			*callStatus = C.RustCallStatus {
				code: C.int8_t(uniffiCallbackResultError),
				errorBuf: FfiConverterTransportErrorINSTANCE.Lower(actualError),
			}
		} else {
			*callStatus = C.RustCallStatus {
				code: C.int8_t(uniffiCallbackUnexpectedResultError),
			}
		}
		return
	}


	
}

var UniffiVTableCallbackInterfaceTransportINSTANCE = C.UniffiVTableCallbackInterfaceTransport {
	uniffiFree: (C.UniffiCallbackInterfaceFree)(C.spoke_connect_ffi_foreign_transport_cgo_dispatchCallbackInterfaceTransportFree),
	uniffiClone: (C.UniffiCallbackInterfaceClone)(C.spoke_connect_ffi_foreign_transport_cgo_dispatchCallbackInterfaceTransportClone),
	send: (C.UniffiCallbackInterfaceTransportMethod0)(C.spoke_connect_ffi_foreign_transport_cgo_dispatchCallbackInterfaceTransportMethod0),
	recv: (C.UniffiCallbackInterfaceTransportMethod1)(C.spoke_connect_ffi_foreign_transport_cgo_dispatchCallbackInterfaceTransportMethod1),
	close: (C.UniffiCallbackInterfaceTransportMethod2)(C.spoke_connect_ffi_foreign_transport_cgo_dispatchCallbackInterfaceTransportMethod2),
}

//export spoke_connect_ffi_foreign_transport_cgo_dispatchCallbackInterfaceTransportFree
func spoke_connect_ffi_foreign_transport_cgo_dispatchCallbackInterfaceTransportFree(handle C.uint64_t) {
	FfiConverterCallbackInterfaceTransportINSTANCE.handleMap.remove(uint64(handle))
}

//export spoke_connect_ffi_foreign_transport_cgo_dispatchCallbackInterfaceTransportClone
func spoke_connect_ffi_foreign_transport_cgo_dispatchCallbackInterfaceTransportClone(handle C.uint64_t) C.uint64_t {
	val, ok := FfiConverterCallbackInterfaceTransportINSTANCE.handleMap.tryGet(uint64(handle))
	if !ok {
		panic(fmt.Errorf("no callback in handle map: %d", handle))
	}
	return C.uint64_t(FfiConverterCallbackInterfaceTransportINSTANCE.handleMap.insert(val))
}

func (c FfiConverterCallbackInterfaceTransport) register() {
	C.uniffi_spoke_connect_fn_init_callback_vtable_transport(&UniffiVTableCallbackInterfaceTransportINSTANCE)
}



type FfiConverterOptionalUint64 struct{}

var FfiConverterOptionalUint64INSTANCE = FfiConverterOptionalUint64{}

func (c FfiConverterOptionalUint64) Lift(rb RustBufferI) *uint64 {
	return LiftFromRustBuffer[*uint64](c, rb)
}

func (_ FfiConverterOptionalUint64) Read(reader io.Reader) *uint64 {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterUint64INSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalUint64) Lower(value *uint64) C.RustBuffer {
	return LowerIntoRustBuffer[*uint64](c, value)
}

func (c FfiConverterOptionalUint64) LowerExternal(value *uint64) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[*uint64](c, value))
}

func (_ FfiConverterOptionalUint64) Write(writer io.Writer, value *uint64) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterUint64INSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalUint64 struct {}

func (_ FfiDestroyerOptionalUint64) Destroy(value *uint64) {
	if value != nil {
		FfiDestroyerUint64{}.Destroy(*value)
	}
}


type FfiConverterOptionalString struct{}

var FfiConverterOptionalStringINSTANCE = FfiConverterOptionalString{}

func (c FfiConverterOptionalString) Lift(rb RustBufferI) *string {
	return LiftFromRustBuffer[*string](c, rb)
}

func (_ FfiConverterOptionalString) Read(reader io.Reader) *string {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterStringINSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalString) Lower(value *string) C.RustBuffer {
	return LowerIntoRustBuffer[*string](c, value)
}

func (c FfiConverterOptionalString) LowerExternal(value *string) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[*string](c, value))
}

func (_ FfiConverterOptionalString) Write(writer io.Writer, value *string) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterStringINSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalString struct {}

func (_ FfiDestroyerOptionalString) Destroy(value *string) {
	if value != nil {
		FfiDestroyerString{}.Destroy(*value)
	}
}


type FfiConverterSequenceString struct{}

var FfiConverterSequenceStringINSTANCE = FfiConverterSequenceString{}

func (c FfiConverterSequenceString) Lift(rb RustBufferI) []string {
	return LiftFromRustBuffer[[]string](c, rb)
}

func (c FfiConverterSequenceString) Read(reader io.Reader) []string {
	length := readInt32(reader)
	if length == 0 {
		return nil
	}
	result := make([]string, 0, length)
	for i := int32(0); i < length; i++ {
		result = append(result, FfiConverterStringINSTANCE.Read(reader))
	}
	return result
}

func (c FfiConverterSequenceString) Lower(value []string) C.RustBuffer {
	return LowerIntoRustBuffer[[]string](c, value)
}

func (c FfiConverterSequenceString) LowerExternal(value []string) ExternalCRustBuffer {
	return RustBufferFromC(LowerIntoRustBuffer[[]string](c, value))
}

func (c FfiConverterSequenceString) Write(writer io.Writer, value []string) {
	if len(value) > math.MaxInt32 {
		panic("[]string is too large to fit into Int32")
	}

	writeInt32(writer, int32(len(value)))
	for _, item := range value {
		FfiConverterStringINSTANCE.Write(writer, item)
	}
}

type FfiDestroyerSequenceString struct {}

func (FfiDestroyerSequenceString) Destroy(sequence []string) {
	for _, value := range sequence {
		FfiDestroyerString{}.Destroy(value)	
	}
}

// Checks that a response echoes the request's `session_id` / `sequence` /
// `request_id` — the three echo fields, flattened to primitives.
func CheckResponseCorrelation(expectedSessionId string, expectedSequence uint64, expectedRequestId string, actualSessionId string, actualSequence uint64, actualRequestId string) error {
	_, _uniffiErr := rustCallWithError[*CoreInvokeError](FfiConverterCoreInvokeError{},func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_spoke_connect_fn_func_check_response_correlation(FfiConverterStringINSTANCE.Lower(expectedSessionId), FfiConverterUint64INSTANCE.Lower(expectedSequence), FfiConverterStringINSTANCE.Lower(expectedRequestId), FfiConverterStringINSTANCE.Lower(actualSessionId), FfiConverterUint64INSTANCE.Lower(actualSequence), FfiConverterStringINSTANCE.Lower(actualRequestId),_uniffiStatus)
		return false
	})
		return _uniffiErr.AsError()
}

// Derive the wire `peer_id` string for a 32-byte Ed25519 public key.
//
// The result matches rust-libp2p `PeerId::to_string()` for the same key
// (locked by the golden-vector tests).
func DerivePeerIdFromEd25519Pubkey(pubkey []byte) (string, error) {
	_uniffiRV, _uniffiErr := rustCallWithError[*CoreError](FfiConverterCoreError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_func_derive_peer_id_from_ed25519_pubkey(FfiConverterBytesINSTANCE.Lower(pubkey),_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

// Whether `op` may be dispatched in a session with
// `negotiated_capabilities`. Fails closed: an unknown `op` has no core-table
// requirement and is not authorized by this gate (hosts answer
// `op_unsupported`).
func DispatchAllowed(op string, negotiatedCapabilities []string) bool {
	return FfiConverterBoolINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.int8_t {
		return C.uniffi_spoke_connect_fn_func_dispatch_allowed(FfiConverterStringINSTANCE.Lower(op), FfiConverterSequenceStringINSTANCE.Lower(negotiatedCapabilities),_uniffiStatus)
	}))
}

// Whether `peer_id` is on the allowlist. Fails closed: an empty allowlist
// rejects every peer.
func IsAllowlisted(allowlist []string, peerId string) bool {
	return FfiConverterBoolINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.int8_t {
		return C.uniffi_spoke_connect_fn_func_is_allowlisted(FfiConverterSequenceStringINSTANCE.Lower(allowlist), FfiConverterStringINSTANCE.Lower(peerId),_uniffiStatus)
	}))
}

// The connect protocol version exchanged in `ConnectHello` (protocol
// version 1 is current).
func ProtocolVersion() uint64 {
	return FfiConverterUint64INSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint64_t {
		return C.uniffi_spoke_connect_fn_func_protocol_version(_uniffiStatus)
	}))
}

// The capability required to dispatch `op`, per the protocol v1 core-op
// table; `None` for product-defined ops.
func RequiredCapability(op string) *string {
	return FfiConverterOptionalStringINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_func_required_capability(FfiConverterStringINSTANCE.Lower(op),_uniffiStatus),
	}
	}))
}

// Sign a hello with a raw Ed25519 secret key (32 bytes), returning the
// signed `ConnectHello` envelope as a JSON string.
//
// `nonce` must meet the wire floor (minLength 16). `host_json` is the
// canonical JSON of the `HostCapabilityManifest` embedded in
// `ConnectHello.host`.
func SignHelloEd25519(secret []byte, nonce string, hostJson string) (string, error) {
	_uniffiRV, _uniffiErr := rustCallWithError[*CoreError](FfiConverterCoreError{},func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer {
		inner: C.uniffi_spoke_connect_fn_func_sign_hello_ed25519(FfiConverterBytesINSTANCE.Lower(secret), FfiConverterStringINSTANCE.Lower(nonce), FfiConverterStringINSTANCE.Lower(hostJson),_uniffiStatus),
	}
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue string
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterStringINSTANCE.Lift(_uniffiRV), nil
		}
}

// Verify a received hello against a 32-byte Ed25519 public key.
//
// `expected_peer_id` is the authenticated remote peer; `hello_json` is the
// JSON string of the received `ConnectHello` envelope. Fails on protocol
// version mismatch, public-key / peer-id binding mismatch, or an invalid
// signature.
func VerifyHelloEd25519(publicKey []byte, expectedPeerId string, helloJson string) error {
	_, _uniffiErr := rustCallWithError[*CoreError](FfiConverterCoreError{},func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_spoke_connect_fn_func_verify_hello_ed25519(FfiConverterBytesINSTANCE.Lower(publicKey), FfiConverterStringINSTANCE.Lower(expectedPeerId), FfiConverterStringINSTANCE.Lower(helloJson),_uniffiStatus)
		return false
	})
		return _uniffiErr.AsError()
}

// Create a back-to-back loopback transport pair (client + server ends).
func NewLoopbackTransportPair() *LoopbackTransportPair {
	return FfiConverterLoopbackTransportPairINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint64_t {
		return C.uniffi_spoke_connect_fn_func_loopback_transport_pair(_uniffiStatus)
	}))
}

func NewMultiPeerRouterFfi() *MultiPeerRouterFfi {
	return FfiConverterMultiPeerRouterFfiINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint64_t {
		return C.uniffi_spoke_connect_fn_func_new_multi_peer_router_ffi(_uniffiStatus)
	}))
}

func ConnectRemoteAdapterFfi(transport Transport, localSeed []byte, localManifestJson string, remotePubkey []byte, allowlist []string, invokeTimeoutMs *uint64) (*RemoteAdapterFfi, error) {
	_uniffiRV, _uniffiErr := rustCallWithError[*FfiError](FfiConverterFfiError{},func(_uniffiStatus *C.RustCallStatus) C.uint64_t {
		return C.uniffi_spoke_connect_fn_func_connect_remote_adapter_ffi(FfiConverterCallbackInterfaceTransportINSTANCE.Lower(transport), FfiConverterBytesINSTANCE.Lower(localSeed), FfiConverterStringINSTANCE.Lower(localManifestJson), FfiConverterBytesINSTANCE.Lower(remotePubkey), FfiConverterSequenceStringINSTANCE.Lower(allowlist), FfiConverterOptionalUint64INSTANCE.Lower(invokeTimeoutMs),_uniffiStatus)
	})
		if _uniffiErr != nil {
			var _uniffiDefaultValue *RemoteAdapterFfi
			return _uniffiDefaultValue, _uniffiErr
		} else {
			return FfiConverterRemoteAdapterFfiINSTANCE.Lift(_uniffiRV), nil
		}
}

