#[cfg(feature = "http-client")]
mod http;
#[cfg(feature = "kv")]
mod kv;

#[cfg(feature = "http-client")]
pub use self::http::HttpCapability;
#[cfg(feature = "kv")]
pub use self::kv::KvCapability;
