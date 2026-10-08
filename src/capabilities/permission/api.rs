//! 入站能力面：别的能力与呈现层只准用这里。

pub use crate::capabilities::permission::domain::permission::{
    validate, validate_override, Granularity, Permissions, PermissionsOverride,
};
