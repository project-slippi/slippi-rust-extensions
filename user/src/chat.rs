/// The default chat messages that we support.
// @TODO: Should these be migrated to a `chat` module eventually?
pub const DEFAULT_CHAT_MESSAGES: [&'static str; 16] = [
    "ggs",
    "one more",
    "brb",
    "good luck",
    "well played",
    "that was fun",
    "thanks",
    "too good",
    "sorry",
    "my b",
    "lol",
    "wow",
    "gotta go",
    "one sec",
    "let's play again later",
    "bad connection",
];

/// Maps the default chat messages to a `Vec<String>`.
pub fn default() -> Vec<String> {
    DEFAULT_CHAT_MESSAGES.iter().map(|msg| msg.to_string()).collect()
}
