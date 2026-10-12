// Mayak's own OAuth applications for the Connect buttons (ideas §14). A device-flow client ID is a
// public value: the device authorization grant (RFC 8628) uses no client secret, so these may live
// in the source. With an empty ID the Connect button says the app is not registered and points to
// the token field; Settings takes an Application ID of the user's own (needed for a self-managed
// GitLab).

/// The GitHub OAuth app (device flow enabled), scope `read:user`: Connect's fallback while the
/// GitHub App below is not registered (repositories and stars, public pull requests only).
pub const GITHUB_CLIENT_ID: &str = "Ov23libuDDcH9cB6PO3g";

/// The read-only GitHub App «Mayak» (device flow enabled, webhook off; repository permissions Pull
/// requests, Commit statuses, Checks and Metadata, read only). Its Client ID, not its App ID; ""
/// until it is registered, and Connect then uses the OAuth app above. A GitHub App asks for no
/// scope: what its token reaches is the app's permissions on the repositories it is installed on.
pub const GITHUB_APP_CLIENT_ID: &str = "";

/// The GitHub App's slug (github.com/apps/<slug>): the «install on repositories» button opens its
/// installation page. "" hides the button.
pub const GITHUB_APP_SLUG: &str = "";

/// The gitlab.com application (non-confidential, device flow), scope `read_api`. A self-managed
/// GitLab needs an application registered on that server: its ID goes in Settings.
pub const GITLAB_COM_CLIENT_ID: &str = "4da7f1579e0154b1c0f89bb0db208bb66cb8c38becbb43ba7cc7b28db7720216";
