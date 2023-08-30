// @generated automatically by Diesel CLI.

diesel::table! {
    item_embedding (item_id) {
        item_id -> Text,
        embedded_text -> Text,
        embedding -> Binary,
    }
}

diesel::table! {
    roam_item (id) {
        id -> Text,
        parent_page_id -> Nullable<Text>,
        parent_item_id -> Nullable<Text>,
        order_in_parent -> Integer,
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

diesel::joinable!(item_embedding -> roam_item (item_id));
diesel::joinable!(roam_item -> roam_page (parent_page_id));

diesel::allow_tables_to_appear_in_same_query!(item_embedding, roam_item, roam_page,);
