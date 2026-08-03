//go:build darwin && amd64

package spoke_connect

/*
#cgo CFLAGS: -I${SRCDIR}
#cgo LDFLAGS: -L${SRCDIR}/../../native/darwin_amd64 -lspoke_connect -Wl,-rpath,${SRCDIR}/../../native/darwin_amd64
*/
import "C"
