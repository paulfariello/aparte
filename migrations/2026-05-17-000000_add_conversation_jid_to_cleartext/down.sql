-- SQLite does not support DROP COLUMN before 3.35; recreate the table without the column.
CREATE TABLE messages_cleartext_backup (
    account    TEXT    NOT NULL,
    message_id TEXT    NOT NULL,
    from_jid   TEXT    NOT NULL,
    body       TEXT    NOT NULL,
    timestamp  TEXT    NOT NULL,
    encrypted  BOOLEAN NOT NULL DEFAULT 1,
    PRIMARY KEY (account, message_id)
);
INSERT INTO messages_cleartext_backup SELECT account, message_id, from_jid, body, timestamp, encrypted FROM messages_cleartext;
DROP TABLE messages_cleartext;
ALTER TABLE messages_cleartext_backup RENAME TO messages_cleartext;
