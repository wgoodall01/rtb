// @generated automatically by Diesel CLI.

diesel::table! {
    roam_item (id) {
        id -> Text,
        parent_page_id -> Nullable<Text>,
        parent_item_id -> Nullable<Text>,
        contents -> Text,
        create_time -> Nullable<BigInt>,
        edit_time -> Nullable<BigInt>,
    }
}

diesel::table! {
    roam_page (title) {
        title -> Text,
        create_time -> Nullable<BigInt>,
        edit_time -> BigInt,
    }
}

diesel::joinable!(roam_item -> roam_page (parent_page_id));

diesel::allow_tables_to_appear_in_same_query!(
    roam_item,
    roam_page,
);
