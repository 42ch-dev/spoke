
package spoke_connect

// #include <spoke_connect.h>
import "C"

import (
	"bytes"
	"fmt"
	"io"
	"unsafe"
	"encoding/binary"
	"math"
	"runtime"
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
// Handshake-level failure (protocol version, peer id binding, …).
type CoreErrorHandshakeFailed struct {
	Reason string
}
// Handshake-level failure (protocol version, peer id binding, …).
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

