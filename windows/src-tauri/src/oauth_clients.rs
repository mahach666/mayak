// Mayak's own OAuth applications for the Connect buttons (ideas §14). A device-flow client ID is a
// public value: the device authorization grant (RFC 8628) uses no client secret, so these may live
// in the source. Empty until the applications are registered; with an empty ID the Connect button
// says so and points to the token field, and Settings takes an Application ID of the user's own.

/// The GitHub OAuth app (device flow enabled), scope `read:user`.
pub const GITHUB_CLIENT_ID: &str = "";

/// The gitlab.com application (non-confidential, device flow), scope `read_api`. A self-managed
/// GitLab needs an application registered on that server: its ID goes in Settings.
pub const GITLAB_COM_CLIENT_ID: &str = "";
