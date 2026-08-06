#[cfg(feature = "ffi-smoke-host")]
pub mod smoke_baseline_adapter;

#[cfg(any(all(test, feature = "remote-adapter"), feature = "ffi-smoke-host"))]
pub mod loopback_oracle {
    mod spoke_connect {
        pub use crate::core;
        pub use crate::remote;
    }
    include!("../../tests/common/loopback_oracle_impl.rs");
}
