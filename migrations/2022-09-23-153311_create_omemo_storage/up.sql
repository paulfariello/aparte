-- Your SQL goes here
CREATE TABLE omemo_own_device (
	id INTEGER PRIMARY KEY NOT NULL,
	account VARCHAR NOT NULL UNIQUE,
	device_id INTEGER NOT NULL,
	current BOOLEAN NOT NULL
);

CREATE TABLE omemo_contact_device (
	id INTEGER PRIMARY KEY NOT NULL,
	account VARCHAR NOT NULL,
	contact VARCHAR NOT NULL,
	device_id INTEGER NOT NULL,
	UNIQUE(account,contact,device_id)

);
