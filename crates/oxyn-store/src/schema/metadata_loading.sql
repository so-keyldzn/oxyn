-- Loading policy never changes a connection's authentication or authority.
CREATE TABLE metadata_loading_policies (
    connection_id TEXT PRIMARY KEY NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK (revision BETWEEN 1 AND 9007199254740991),
    payload TEXT NOT NULL
) STRICT;
