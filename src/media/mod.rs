//! Provider-neutral encoded audio, decoding and cache policy.
//! Spotify keeps its native librespot loader behind the same cache budget and
//! output contracts. HTTP providers supply authenticated requests and identities.
pub mod cache;
pub mod decode;
pub mod http;
pub mod pipeline;
