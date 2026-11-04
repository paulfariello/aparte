CREATE TABLE archives (
    account          TEXT    NOT NULL,
    message_id       TEXT    NOT NULL,
    conversation_jid TEXT    NOT NULL DEFAULT '',
    from_jid         TEXT    NOT NULL,
    body_enc         BLOB    NOT NULL,
    timestamp        TEXT    NOT NULL,
    encrypted        BOOLEAN NOT NULL DEFAULT 1,
    PRIMARY KEY (account, message_id)
);

CREATE TABLE account_crypto_config (
    account     TEXT NOT NULL PRIMARY KEY,
    kdf_salt    BLOB NOT NULL,
    wrapped_dek BLOB NOT NULL
);
