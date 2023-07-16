create table item_embedding (
	item_id text not null primary key,
	embedded_text text not null,
	embedding blob not null,

	foreign key (item_id) references roam_item(id)
)

