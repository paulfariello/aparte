-- Your SQL goes here
CREATE TABLE omemo_device (
	id INTEGER PRIMARY KEY NOT NULL,
	account VARCHAR NOT NULL UNIQUE,
	device_id INTEGER NOT NULL
)
