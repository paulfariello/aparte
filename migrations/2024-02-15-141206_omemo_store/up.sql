CREATE TABLE omemo_identity (
	identity_pk INTEGER PRIMARY KEY NOT NULL,
	account VARCHAR NOT NULL,
	user_id VARCHAR NOT NULL,
	device_id BIGINT NOT NULL,
	identity BLOB NOT NULL,
	UNIQUE(account, user_id, device_id)
);

CREATE TABLE omemo_session (
	session_pk INTEGER PRIMARY KEY NOT NULL,
	account VARCHAR NOT NULL,
	user_id VARCHAR NOT NULL,
	device_id BIGINT NOT NULL,
	session BLOB NOT NULL,
	UNIQUE(account, user_id, device_id)
);

CREATE TABLE omemo_pre_key (
	pre_key_pk INTEGER PRIMARY KEY NOT NULL,
	account VARCHAR NOT NULL,
	pre_key_id BIGINT NOT NULL,
	pre_key BLOB NOT NULL,
	UNIQUE(account, pre_key_id)
);

CREATE TABLE omemo_signed_pre_key (
	signed_pre_key_pk INTEGER PRIMARY KEY NOT NULL,
	account VARCHAR NOT NULL,
	signed_pre_key_id BIGINT NOT NULL,
	signed_pre_key BLOB NOT NULL,
	UNIQUE(account, signed_pre_key_id)
);

CREATE TABLE omemo_sender_key (
	sender_key_pk INTEGER PRIMARY KEY NOT NULL,
	account VARCHAR NOT NULL,
	sender_id VARCHAR NOT NULL,
	device_id BIGINT NOT NULL,
	distribution_id BLOB NOT NULL,
	sender_key BLOB NOT NULL,
	UNIQUE(account, sender_id, device_id, distribution_id)
);
