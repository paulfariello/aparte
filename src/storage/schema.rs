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
    omemo_contact_prekey (contact_prekey_pk) {
        contact_prekey_pk -> Integer,
        contact_device_fk -> Integer,
        id -> BigInt,
        data -> Binary,
    }
}

diesel::table! {
    omemo_own_device (own_device_pk) {
        own_device_pk -> Integer,
        account -> Text,
        id -> BigInt,
        current -> Bool,
    }
}

diesel::joinable!(omemo_contact_prekey -> omemo_contact_device (contact_device_fk));

diesel::allow_tables_to_appear_in_same_query!(
    omemo_contact_device,
    omemo_contact_prekey,
    omemo_own_device,
);
