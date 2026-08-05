#[cfg(any(test, all(feature = "ffi", feature = "remote-adapter")))]
pub mod loopback_oracle {
    mod spoke_connect {
        pub use crate::core;
        pub use crate::remote;
    }
    include!("../../tests/common/loopback_oracle_impl.rs");
}
