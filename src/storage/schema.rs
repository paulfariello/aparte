// @generated automatically by Diesel CLI.

diesel::table! {
    omemo_contact_device (contact_device_pk) {
        contact_device_pk -> Integer,
        account -> Text,
        contact -> Text,
        id -> BigInt,
    }
}

diesel::table! {
    omemo_identity (identity_pk) {
        identity_pk -> Integer,
        account -> Text,
        user_id -> Text,
        device_id -> BigInt,
        identity -> Binary,
    }
}

diesel::table! {
    omemo_own_device (own_device_pk) {
        own_device_pk -> Integer,
        account -> Text,
        id -> BigInt,
        identity -> Nullable<Binary>,
    }
}

diesel::table! {
    omemo_pre_key (pre_key_pk) {
        pre_key_pk -> Integer,
        account -> Text,
        pre_key_id -> BigInt,
        pre_key -> Binary,
    }
}

diesel::table! {
    omemo_sender_key (sender_key_pk) {
        sender_key_pk -> Integer,
        account -> Text,
        sender_id -> Text,
        device_id -> BigInt,
        distribution_id -> Binary,
        sender_key -> Binary,
    }
}

diesel::table! {
    omemo_session (session_pk) {
        session_pk -> Integer,
        account -> Text,
        user_id -> Text,
        device_id -> BigInt,
        session -> Binary,
    }
}

diesel::table! {
    omemo_signed_pre_key (signed_pre_key_pk) {
        signed_pre_key_pk -> Integer,
        account -> Text,
        signed_pre_key_id -> BigInt,
        signed_pre_key -> Binary,
    }
}

diesel::allow_tables_to_appear_in_same_query!(
    omemo_contact_device,
    omemo_identity,
    omemo_own_device,
    omemo_pre_key,
    omemo_sender_key,
    omemo_session,
    omemo_signed_pre_key,
);
