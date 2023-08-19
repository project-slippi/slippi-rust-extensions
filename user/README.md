# slippi-user
This crate implements authentication handling and user management for players. It consists of three main components:

- `UserManager`: a thread-safe type that can be cloned and passed around. This can be used to access the current user.
- `UserInfo`: A type that reflects various user properties (play keys, ids, etc).
- `UserInfoWatcher`: A background thread that watches for `user.json` files to load and parse.

The `UserManager` and `UserInfo` structs are generally the only pieces that one would need to interact with.
