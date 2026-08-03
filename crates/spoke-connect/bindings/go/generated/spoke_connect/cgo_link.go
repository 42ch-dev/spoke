//go:build darwin && arm64

package spoke_connect

/*
#cgo CFLAGS: -I${SRCDIR}
#cgo LDFLAGS: -L${SRCDIR}/../../native/darwin_arm64 -lspoke_connect -Wl,-rpath,${SRCDIR}/../../native/darwin_arm64
*/
import "C"
