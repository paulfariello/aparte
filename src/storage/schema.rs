// @generated automatically by Diesel CLI.

diesel::table! {
    omemo_contact_device (id) {
        id -> Integer,
        account -> Text,
        contact -> Text,
        device_id -> Integer,
    }
}

diesel::table! {
    omemo_own_device (id) {
        id -> Integer,
        account -> Text,
        device_id -> Integer,
        current -> Bool,
    }
}

diesel::allow_tables_to_appear_in_same_query!(omemo_contact_device, omemo_own_device,);
