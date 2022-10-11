-- Your SQL goes here
CREATE TABLE omemo_own_device (
	own_device_pk INTEGER PRIMARY KEY NOT NULL,
	account VARCHAR NOT NULL UNIQUE,
	id BIGINT NOT NULL,
	current BOOLEAN NOT NULL
);

CREATE TABLE omemo_contact_device (
	contact_device_pk INTEGER PRIMARY KEY NOT NULL,
	account VARCHAR NOT NULL,
	contact VARCHAR NOT NULL,
	id BIGINT NOT NULL,
	UNIQUE(account, contact, id)

);

CREATE TABLE omemo_contact_prekey (
	contact_prekey_pk INTEGER PRIMARY KEY NOT NULL,
	contact_device_fk INTEGER NOT NULL,
	id BIGINT NOT NULL,
	data BLOB NOT NULL,
	UNIQUE(contact_device_fk, id),
	FOREIGN KEY(contact_device_fk) REFERENCES omemo_contact_device(contact_device_pk)

)
