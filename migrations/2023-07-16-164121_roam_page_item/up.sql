create table roam_page (
	title text not null primary key,
	create_time big integer,
	edit_time big integer not null
);

create table roam_item (
	id text not null primary key,

	parent_page_id text null references roam_page(title) on delete cascade,
	parent_item_id text null references roam_item(id) on delete cascade,
	order_in_parent integer not null,

	contents text not null,

	create_time big integer,
	edit_time big integer,

	-- Check that parent_page_id XOR parent_item_id is set.
	check ((parent_page_id is null) != (parent_item_id is null)),

	-- Check that order is non-negative.
	check (order_in_parent >= 0)
);
