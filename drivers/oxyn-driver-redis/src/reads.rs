//! The commands the driver knows to be reads, compiled in.
//!
//! # Why the server's word is not enough
//!
//! `COMMAND` flags a command `readonly`, and a server answers whatever it
//! wants: a fork, a proxy or a hostile endpoint can flag `SET`, `CONFIG SET`
//! or `FLUSHALL` `readonly`. Both the `PolicyGate` and the read-only guard of
//! an execution would then send a write to a production server as a read.
//! And a genuine server is not right either: on Redis 8.10.2 the search
//! module flags `FT.CURSOR DEL` and the `_FT.DEBUG` subcommands that
//! force a garbage collection or change a limit `readonly`.
//!
//! So `Read` is proven only for a command of [`READS`] that the server's table
//! **also** flags `readonly`, outside `@dangerous`: the table can withdraw a
//! read, never grant one
//! ([ADR-0061](../../../docs/adr/0061-redis-driver-speaks-resp3-itself.md)).
//! A command missing from the list is at worst `Unknown`, which asks for a
//! confirmation it did not need; one wrongly on it would run a write unasked.
//!
//! # Where the list comes from
//!
//! `COMMAND` on Redis 8.10.2 and Valkey 9.1.2, observed on 2026-10-24
//! ([RESEARCH-NOTES](../../../docs/RESEARCH-NOTES.md)): every command both
//! flag `readonly` outside `@dangerous`, less `LOLWUT`, which reads no data;
//! `SDIFFCARD` and `SUNIONCARD`, which only Redis has; and the read commands
//! of the modules Redis loads — JSON, search, time series, probabilistic,
//! vector sets — that the server flags `readonly` and whose purpose is to
//! read. Left out: what Redis 8.10.2 alone flags and the driver has not
//! checked (`DIGEST`, the `AR*` array commands), the debugging and
//! cluster-management
//! commands of the modules, and the scripts: `EVAL`, `EVALSHA` and `FCALL`
//! run what their body says, while `EVAL_RO`, `EVALSHA_RO` and `FCALL_RO` are
//! refused a write by the server.

/// The commands proven read-only, lowercase; a subcommand is written
/// `container|subcommand`, as `COMMAND` names it. Sorted, for the binary
/// search.
pub const READS: &[&str] = &[
    "bf.card",
    "bf.exists",
    "bf.info",
    "bf.mexists",
    "bitcount",
    "bitfield_ro",
    "bitpos",
    "cf.count",
    "cf.exists",
    "cf.info",
    "cf.mexists",
    "cms.info",
    "cms.query",
    "dbsize",
    "dump",
    "eval_ro",
    "evalsha_ro",
    "exists",
    "expiretime",
    "fcall_ro",
    "ft._list",
    "ft.aggregate",
    "ft.config|get",
    "ft.dictdump",
    "ft.explain",
    "ft.explaincli",
    "ft.info",
    "ft.profile",
    "ft.search",
    "ft.spellcheck",
    "ft.sugget",
    "ft.suglen",
    "ft.syndump",
    "geodist",
    "geohash",
    "geopos",
    "georadius_ro",
    "georadiusbymember_ro",
    "geosearch",
    "get",
    "getbit",
    "getrange",
    "hexists",
    "hexpiretime",
    "hget",
    "hgetall",
    "hkeys",
    "hlen",
    "hmget",
    "hpexpiretime",
    "hpttl",
    "hrandfield",
    "hscan",
    "hstrlen",
    "httl",
    "hvals",
    "json.arrindex",
    "json.arrlen",
    "json.get",
    "json.mget",
    "json.objkeys",
    "json.objlen",
    "json.resp",
    "json.strlen",
    "json.type",
    "lcs",
    "lindex",
    "llen",
    "lpos",
    "lrange",
    "memory|usage",
    "mget",
    "object|encoding",
    "object|freq",
    "object|idletime",
    "object|refcount",
    "pexpiretime",
    "pfcount",
    "pttl",
    "randomkey",
    "scan",
    "scard",
    "sdiff",
    "sdiffcard",
    "sinter",
    "sintercard",
    "sismember",
    "smembers",
    "smismember",
    "srandmember",
    "sscan",
    "strlen",
    "substr",
    "sunion",
    "sunioncard",
    "tdigest.byrank",
    "tdigest.byrevrank",
    "tdigest.cdf",
    "tdigest.info",
    "tdigest.max",
    "tdigest.min",
    "tdigest.quantile",
    "tdigest.rank",
    "tdigest.revrank",
    "tdigest.trimmed_mean",
    "topk.count",
    "topk.info",
    "topk.list",
    "topk.query",
    "touch",
    "ts.get",
    "ts.info",
    "ts.mget",
    "ts.mrange",
    "ts.mrevrange",
    "ts.queryindex",
    "ts.range",
    "ts.revrange",
    "ttl",
    "type",
    "vcard",
    "vdim",
    "vemb",
    "vgetattr",
    "vinfo",
    "vismember",
    "vlinks",
    "vrandmember",
    "vrange",
    "vsim",
    "xinfo|consumers",
    "xinfo|groups",
    "xinfo|stream",
    "xlen",
    "xpending",
    "xrange",
    "xread",
    "xrevrange",
    "zcard",
    "zcount",
    "zdiff",
    "zinter",
    "zintercard",
    "zlexcount",
    "zmscore",
    "zrandmember",
    "zrange",
    "zrangebylex",
    "zrangebyscore",
    "zrank",
    "zrevrange",
    "zrevrangebylex",
    "zrevrangebyscore",
    "zrevrank",
    "zscan",
    "zscore",
    "zunion",
];

/// Whether `command`, or `command|subcommand`, is on [`READS`]. Both are
/// lowercase.
#[must_use]
pub fn is_read(command: &str, subcommand: Option<&str>) -> bool {
    let name = match subcommand {
        Some(sub) => format!("{command}|{sub}"),
        None => command.to_owned(),
    };
    READS.binary_search(&name.as_str()).is_ok()
}

/// Whether a command line names a command of [`READS`], without the
/// server's table: the argv alone, its first word or its first two.
///
/// What the read-only guard of an execution checks on top of the intent, so
/// that a table which lies cannot open it alone.
#[must_use]
pub fn names_a_read(argv: &[Vec<u8>]) -> bool {
    let word = |index: usize| {
        argv.get(index)
            .filter(|word| word.is_ascii())
            .map(|word| String::from_utf8_lossy(word).to_ascii_lowercase())
    };
    let Some(command) = word(0) else {
        return false;
    };
    is_read(&command, None) || word(1).is_some_and(|sub| is_read(&command, Some(&sub)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_sorted_and_without_duplicates() {
        assert!(READS.windows(2).all(|pair| matches!(pair, [a, b] if a < b)));
    }

    #[test]
    fn no_command_that_writes_or_runs_a_script_is_on_the_list() {
        for name in [
            "set",
            "del",
            "flushall",
            "flushdb",
            "config|set",
            "config|get",
            "eval",
            "evalsha",
            "fcall",
            "debug",
            "shutdown",
            "keys",
            "info",
            "sort",
            "migrate",
            "ft.cursor|del",
            "json.debug",
            "acl|setuser",
        ] {
            assert!(READS.binary_search(&name).is_err(), "{name}");
        }
    }

    #[test]
    fn a_line_names_a_read_by_its_command_or_its_subcommand() {
        let line = |words: &[&str]| -> Vec<Vec<u8>> {
            words.iter().map(|word| word.as_bytes().to_vec()).collect()
        };
        assert!(names_a_read(&line(&["GET", "k"])));
        assert!(names_a_read(&line(&["Object", "ENCODING", "k"])));
        assert!(!names_a_read(&line(&["OBJECT", "HELP"])));
        assert!(!names_a_read(&line(&["CONFIG", "SET", "x", "y"])));
        assert!(!names_a_read(&line(&["SET", "get", "v"])));
        assert!(!names_a_read(&[b"G\xc9T".to_vec()]));
        assert!(!names_a_read(&[]));
    }
}
