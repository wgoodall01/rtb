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

# Run a similarity search, copying results in Roam markdown format to clipboard.
$ cargo run -rq -- search -o >(pbcopy) -k 16 "Issues with speculative execution"
```

### Results

<img width="982" alt="image" src="https://github.com/wgoodall01/rtb/assets/15006576/1cd8c466-d0c2-4d71-8243-00dc79e32660">

<img width="1115" alt="image" src="https://github.com/wgoodall01/rtb/assets/15006576/d236b8e2-d0ef-42f2-99d0-6cc872f993b2">
(some children expanded as references)

