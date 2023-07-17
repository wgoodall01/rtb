# rtb
Roam Third Brain: vector search over RoamResearch blocks

### Usage

```bash
$ cargo run -q -- --help
Usage: rtb [OPTIONS] <COMMAND>

Commands:
  import
  update-embeddings
  search
  help               Print this message or the help of the given subcommand(s)

Options:
      --db <DB>  Path to the database file [default: rtb.db]
  -v, --verbose  Increase logging verbosity
  -h, --help     Print help

# Import your Roam graph
$ cargo run -rq -- import ~/path/to/RoamResearch/json/export.json
2023-07-17T14:33:30.035491Z  INFO Load RoamResearch export{file="/Users/wgoodall01/Desktop/w01.json"}: new
2023-07-17T14:33:30.102134Z  INFO Load RoamResearch export{file="/Users/wgoodall01/Desktop/w01.json"}: close time.busy=0.00ns time.idle=66.7ms
2023-07-17T14:33:30.102615Z  INFO Loaded Roam export num_pages=6864 num_children=40494
2023-07-17T14:33:30.102635Z  INFO Load export into database: new
2023-07-17T14:33:30.107943Z  INFO Load export into database: new_pages=1 new_items=18 total_pages=6864
2023-07-17T14:33:30.493026Z  INFO Load export into database: new_pages=257 new_items=1378 total_pages=6864
< ... >
2023-07-17T14:33:38.797013Z  INFO Load export into database: new_pages=6657 new_items=38437 total_pages=6864
2023-07-17T14:33:39.107165Z  INFO Load export into database: close time.busy=9.00s time.idle=19.9µs

# Take a peek at the database
$ echo 'select '\
  '(select count(*) from roam_page) as page_count, ' \
  '(select count(*) from roam_item) as item_count, ' \
  '(select avg(length(contents)) from roam_item) as mean_item_char_count;' \
  | sqlite3 rtb.db -column -header
page_count  item_count  mean_item_char_count
----------  ----------  --------------------
6864        40487       74.9887124262109

# Generate embeddings for each block
$ export OPENAI_API_KEY="sk-xxxxx"
$ cargo run -rq -- update-embeddings
2023-07-17T14:34:35.917469Z  INFO exec_update_embeddings: new
2023-07-17T14:34:40.485073Z  INFO exec_update_embeddings: Updated batch embeddings_updated=512 total_to_embed=39873
2023-07-17T14:34:40.966339Z  INFO exec_update_embeddings: Updated batch embeddings_updated=1024 total_to_embed=39873
2023-07-17T14:34:41.020282Z  INFO exec_update_embeddings: Updated batch embeddings_updated=1536 total_to_embed=39873
< ... >

# Run a similarity search, copying results in Roam markdown format to clipboard.
$ cargo run -rq -- search -o >(pbcopy) -k 16 "Issues with speculative execution"
2023-07-17T04:24:16.146174Z  INFO exec_search: new
2023-07-17T04:24:16.146219Z  INFO exec_search:Load item embeddings: new
2023-07-17T04:24:16.316316Z  INFO exec_search:Load item embeddings: close time.busy=170ms time.idle=3.67µs
2023-07-17T04:24:16.427368Z  INFO exec_search:Embed query: new
2023-07-17T04:24:16.842536Z  INFO exec_search:Embed query: close time.busy=415ms time.idle=15.6µs
2023-07-17T04:24:16.842616Z  INFO exec_search:Finding k-most-similar: new
2023-07-17T04:24:16.904109Z  INFO exec_search:Finding k-most-similar: close time.busy=61.5ms time.idle=11.2µs
2023-07-17T04:24:16.904140Z  INFO exec_search: result_count=16
2023-07-17T04:24:16.904144Z  INFO exec_search: similarity=-0.8505694 id=casKRVgMz
< ... >
2023-07-17T04:24:16.904288Z  INFO exec_search: similarity=-0.80927974 id=NUWb0CzeY
2023-07-17T04:24:16.904507Z  INFO exec_search: close time.busy=345ms time.idle=413ms
```

### Results

<img width="982" alt="image" src="https://github.com/wgoodall01/rtb/assets/15006576/1cd8c466-d0c2-4d71-8243-00dc79e32660">

<img width="1115" alt="image" src="https://github.com/wgoodall01/rtb/assets/15006576/d236b8e2-d0ef-42f2-99d0-6cc872f993b2">
(some children expanded as references)

