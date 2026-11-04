CREATE TABLE omemo_own_device (
	own_device_pk INTEGER PRIMARY KEY NOT NULL,
	account VARCHAR NOT NULL UNIQUE,
	id BIGINT NOT NULL,
	identity BLOB
);

CREATE TABLE omemo_contact_device (
	contact_device_pk INTEGER PRIMARY KEY NOT NULL,
	account VARCHAR NOT NULL,
	contact VARCHAR NOT NULL,
	id BIGINT NOT NULL,
	UNIQUE(account, contact, id)
);
