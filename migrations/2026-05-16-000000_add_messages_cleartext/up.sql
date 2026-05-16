CREATE TABLE messages_cleartext (
    account    TEXT    NOT NULL,
    message_id TEXT    NOT NULL,
    from_jid   TEXT    NOT NULL,
    body       TEXT    NOT NULL,
    timestamp  TEXT    NOT NULL,
    encrypted  BOOLEAN NOT NULL DEFAULT 1,
    PRIMARY KEY (account, message_id)
);
