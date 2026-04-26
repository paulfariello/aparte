CREATE TABLE omemo_muc_room (
    account TEXT NOT NULL,
    room    TEXT NOT NULL,
    PRIMARY KEY (account, room)
);
