### Added

- Review with Cursor. Sign in to your Cursor plan in the browser, or add a user API key from your
  Cursor dashboard, from onboarding, Settings or the account menu of the model picker. The picker
  offers the models your Cursor plan lists when you sign in. Cursor accounts take part in
  priority and failover like the other providers; Settings doesn't show Cursor's limits.
- A Cursor browser sign-in is not renewed by Cursor's CLI: when it expires, the account shows as
  signed out until you sign in again. An API key doesn't expire.

### Security

- Review runners may now reach `api2.cursor.sh` and `api3.cursor.sh`. Every Cursor tool call goes
  through the same read-only tool guard as Claude's, from system hooks installed read-only in the
  runner image; a guard that fails denies the call.
