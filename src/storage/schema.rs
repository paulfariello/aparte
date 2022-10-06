// @generated automatically by Diesel CLI.

diesel::table! {
    omemo_device (id) {
        id -> Integer,
        account -> Text,
        device_id -> Integer,
    }
}
