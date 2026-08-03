//go:build linux && amd64

package spoke_connect

/*
#cgo CFLAGS: -I${SRCDIR}
#cgo LDFLAGS: -L${SRCDIR}/../../native/linux_amd64 -lspoke_connect -Wl,-rpath,${SRCDIR}/../../native/linux_amd64
*/
import "C"
