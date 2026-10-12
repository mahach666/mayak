// Mayak's own OAuth applications for the Connect buttons (ideas §14). A device-flow client ID is a
// public value: the device authorization grant (RFC 8628) uses no client secret, so these may live
// in the source. With an empty ID the Connect button says the app is not registered and points to
// the token field; Settings takes an Application ID of the user's own (needed for a self-managed
// GitLab).

/// The GitHub OAuth app (device flow enabled), scope `read:user`.
pub const GITHUB_CLIENT_ID: &str = "Ov23libuDDcH9cB6PO3g";

/// The gitlab.com application (non-confidential, device flow), scope `read_api`. A self-managed
/// GitLab needs an application registered on that server: its ID goes in Settings.
pub const GITLAB_COM_CLIENT_ID: &str = "4da7f1579e0154b1c0f89bb0db208bb66cb8c38becbb43ba7cc7b28db7720216";
