//go:build windows && amd64

package spoke_connect

/*
#cgo CFLAGS: -I${SRCDIR}
#cgo LDFLAGS: -L${SRCDIR}/../../native/windows_amd64 -lspoke_connect
*/
import "C"
