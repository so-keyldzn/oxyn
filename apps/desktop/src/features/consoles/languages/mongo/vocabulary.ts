// The words the MongoDB console completes, each with the short hint shown
// beside it. Read from the MongoDB manual on 2026-10-23 (docs/RESEARCH-NOTES.md,
// « MongoDB client library and server »): the command pages, the query, update
// and pipeline-stage indexes, and `$group`'s accumulators.
//
// A hint, never a judgement: what a command does to the data is decided by
// `oxyn-query`'s tables in Rust, and a name missing here is still sent as
// typed. That is why the lists are a choice — the commands a console is used
// for — and why the operators that run JavaScript on the server (`$where`,
// `$function`, `$accumulator`) are not offered: Oxyn holds them for approval.

/** A word to complete, and what the manual says it is for. */
export interface Word {
  name: string
  info: string
}

/** What a command does, as the classifier reads it. */
export type Effect = "read" | "write" | "structure"

export interface CommandWord extends Word {
  effect: Effect
  /** Its first value is a collection of the database. */
  collection: boolean
  /** Its other fields, in the manual's order. */
  fields: ReadonlyArray<Word>
}

const COMMENT: Word = {
  name: "comment",
  info: "Attached to the command in logs, the profiler and currentOp.",
}
const MAX_TIME: Word = {
  name: "maxTimeMS",
  info: "Time limit in milliseconds, after which the server stops it.",
}
const COLLATION: Word = {
  name: "collation",
  info: "Language-specific rules for string comparison.",
}
const HINT: Word = {
  name: "hint",
  info: "The index to use, by name or key pattern.",
}
const LET: Word = {
  name: "let",
  info: "Variables the command reads as $$<name>.",
}
const READ_CONCERN: Word = {
  name: "readConcern",
  info: "The read concern level of the query.",
}
const WRITE_CONCERN: Word = {
  name: "writeConcern",
  info: "The acknowledgement the write waits for.",
}
const ORDERED: Word = {
  name: "ordered",
  info: "Stop at the first failed statement (true, the default).",
}
const BYPASS: Word = {
  name: "bypassDocumentValidation",
  info: "Write documents that fail the collection's validator.",
}

export const COMMANDS: ReadonlyArray<CommandWord> = [
  {
    name: "find",
    info: "Documents of a collection or view that match a filter.",
    effect: "read",
    collection: true,
    fields: [
      { name: "filter", info: "The query predicate; all documents if absent." },
      { name: "sort", info: "The order of the results." },
      { name: "projection", info: "The fields each document returns." },
      HINT,
      { name: "skip", info: "Documents to skip first." },
      { name: "limit", info: "At most this many documents." },
      { name: "batchSize", info: "Documents per batch the server returns." },
      { name: "singleBatch", info: "Close the cursor after the first batch." },
      COMMENT,
      MAX_TIME,
      READ_CONCERN,
      { name: "max", info: "Exclusive upper bound for the hinted index." },
      { name: "min", info: "Inclusive lower bound for the hinted index." },
      { name: "returnKey", info: "Return only the index keys." },
      { name: "showRecordId", info: "Add a $recordId field to each document." },
      {
        name: "allowPartialResults",
        info: "Sharded: partial results when a shard is unavailable.",
      },
      COLLATION,
      { name: "allowDiskUse", info: "Allow or forbid temporary disk writes." },
      LET,
    ],
  },
  {
    name: "aggregate",
    info: "Runs a pipeline of stages on a collection or view.",
    effect: "read",
    collection: true,
    fields: [
      { name: "pipeline", info: "The stages, in order." },
      {
        name: "cursor",
        info: "Cursor options such as batchSize; required, {} will do.",
      },
      { name: "explain", info: "Return how the pipeline would run." },
      { name: "allowDiskUse", info: "Allow or forbid temporary disk writes." },
      {
        name: "allowPartialResults",
        info: "Sharded: partial results when a shard is unavailable.",
      },
      MAX_TIME,
      BYPASS,
      READ_CONCERN,
      COLLATION,
      HINT,
      COMMENT,
      WRITE_CONCERN,
      LET,
    ],
  },
  {
    name: "count",
    info: "The number of documents that match a query.",
    effect: "read",
    collection: true,
    fields: [
      { name: "query", info: "The query predicate." },
      { name: "limit", info: "Count at most this many." },
      { name: "skip", info: "Documents to skip before counting." },
      HINT,
      READ_CONCERN,
      MAX_TIME,
      COLLATION,
      COMMENT,
    ],
  },
  {
    name: "distinct",
    info: "The distinct values of a field.",
    effect: "read",
    collection: true,
    fields: [
      { name: "key", info: "The field whose distinct values are returned." },
      { name: "query", info: "The query predicate." },
      READ_CONCERN,
      COLLATION,
      COMMENT,
      HINT,
      MAX_TIME,
    ],
  },
  {
    name: "insert",
    info: "Inserts documents.",
    effect: "write",
    collection: true,
    fields: [
      { name: "documents", info: "The documents to insert." },
      ORDERED,
      MAX_TIME,
      WRITE_CONCERN,
      BYPASS,
      COMMENT,
    ],
  },
  {
    name: "update",
    info: "Updates the documents each statement matches.",
    effect: "write",
    collection: true,
    fields: [
      { name: "updates", info: "The update statements: q, u…" },
      ORDERED,
      MAX_TIME,
      WRITE_CONCERN,
      BYPASS,
      COMMENT,
      LET,
    ],
  },
  {
    name: "delete",
    info: "Deletes the documents each statement matches.",
    effect: "write",
    collection: true,
    fields: [
      { name: "deletes", info: "The delete statements: q, limit…" },
      COMMENT,
      LET,
      ORDERED,
      WRITE_CONCERN,
      MAX_TIME,
    ],
  },
  {
    name: "findAndModify",
    info: "Updates or removes one document and returns it.",
    effect: "write",
    collection: true,
    fields: [
      { name: "query", info: "Selects the document." },
      { name: "sort", info: "Which document, when several match." },
      { name: "remove", info: "Remove the document instead of updating it." },
      { name: "update", info: "The update document or pipeline." },
      { name: "new", info: "Return the modified document, not the original." },
      { name: "fields", info: "The fields to return." },
      { name: "upsert", info: "Insert a document when none matches." },
      BYPASS,
      WRITE_CONCERN,
      MAX_TIME,
      COLLATION,
      {
        name: "arrayFilters",
        info: "Which array elements $[<identifier>] updates.",
      },
      HINT,
      COMMENT,
      LET,
    ],
  },
  {
    name: "listCollections",
    info: "The collections and views of the database.",
    effect: "read",
    collection: false,
    fields: [
      { name: "filter", info: "A predicate on what the command returns." },
      { name: "nameOnly", info: "Names and types only, without options." },
      {
        name: "authorizedCollections",
        info: "Only the collections the user may act on.",
      },
      COMMENT,
    ],
  },
  {
    name: "listIndexes",
    info: "The indexes of a collection.",
    effect: "read",
    collection: true,
    fields: [],
  },
  {
    name: "dbStats",
    info: "Storage statistics of the database.",
    effect: "read",
    collection: false,
    fields: [],
  },
  {
    name: "explain",
    info: "How the server would run a command.",
    effect: "read",
    collection: false,
    fields: [
      {
        name: "verbosity",
        info: "queryPlanner, executionStats or allPlansExecution (default).",
      },
      COMMENT,
    ],
  },
  {
    name: "createIndexes",
    info: "Builds indexes on a collection.",
    effect: "structure",
    collection: true,
    fields: [
      { name: "indexes", info: "The index specifications: key, name…" },
      WRITE_CONCERN,
      {
        name: "commitQuorum",
        info: "Voting members that must finish the build.",
      },
      COMMENT,
    ],
  },
  {
    name: "dropIndexes",
    info: "Drops indexes of a collection.",
    effect: "structure",
    collection: true,
    fields: [],
  },
  {
    name: "create",
    info: "Creates a collection or a view.",
    effect: "structure",
    collection: true,
    fields: [],
  },
  {
    name: "drop",
    info: "Drops a collection and its indexes.",
    effect: "structure",
    collection: true,
    fields: [],
  },
  {
    name: "collMod",
    info: "Changes options of a collection or view.",
    effect: "structure",
    collection: true,
    fields: [],
  },
]

/** A field every command takes: the database it runs in (ADR-0063). */
export const DATABASE_FIELD: Word = {
  name: "$db",
  info: "The database the command runs in, instead of the console's.",
}

/** The fields of one statement of `update`'s `updates`. */
export const UPDATE_STATEMENT: ReadonlyArray<Word> = [
  { name: "q", info: "Selects the documents to update." },
  { name: "u", info: "The update document or pipeline." },
  { name: "c", info: "Constants for an update pipeline." },
  { name: "upsert", info: "Insert a document when none matches." },
  { name: "multi", info: "Update every match, not only the first." },
  COLLATION,
  {
    name: "arrayFilters",
    info: "Which array elements $[<identifier>] updates.",
  },
  HINT,
  { name: "sort", info: "Which document a single update picks." },
]

/** The fields of one statement of `delete`'s `deletes`. */
export const DELETE_STATEMENT: ReadonlyArray<Word> = [
  { name: "q", info: "Selects the documents to delete." },
  { name: "limit", info: "0 deletes every match, 1 deletes one." },
  COLLATION,
  HINT,
]

/** The fields of one index specification of `createIndexes`. */
export const INDEX_SPECIFICATION: ReadonlyArray<Word> = [
  { name: "key", info: "The fields and their direction or type." },
  { name: "name", info: "A name unique among the collection's indexes." },
  { name: "unique", info: "Refuse two documents with the same key." },
  {
    name: "partialFilterExpression",
    info: "Index only the documents that match.",
  },
  { name: "sparse", info: "Skip documents without the field." },
  { name: "expireAfterSeconds", info: "TTL: delete documents this old." },
  { name: "hidden", info: "Kept up to date, not used by the planner." },
  COLLATION,
]

/** Operators that open a predicate: at the top of a filter. */
export const QUERY_TOP: ReadonlyArray<Word> = [
  { name: "$and", info: "Every clause matches." },
  { name: "$or", info: "At least one clause matches." },
  { name: "$nor", info: "No clause matches." },
  { name: "$expr", info: "An aggregation expression as the predicate." },
  { name: "$jsonSchema", info: "Validates documents against a JSON Schema." },
]

/** Operators that test one field: `{"age": {"$gt": 30}}`. */
export const QUERY_FIELD: ReadonlyArray<Word> = [
  { name: "$eq", info: "Equal to a value." },
  { name: "$ne", info: "Not equal to a value." },
  { name: "$gt", info: "Greater than a value." },
  { name: "$gte", info: "Greater than or equal to a value." },
  { name: "$lt", info: "Less than a value." },
  { name: "$lte", info: "Less than or equal to a value." },
  { name: "$in", info: "Equal to any value of an array." },
  { name: "$nin", info: "Equal to no value of an array." },
  { name: "$exists", info: "The field is present (true) or absent." },
  { name: "$type", info: "The field is of a BSON type." },
  { name: "$regex", info: "Matches a regular expression." },
  { name: "$mod", info: "The remainder of a division." },
  { name: "$not", info: "Inverts an operator expression." },
  { name: "$all", info: "The array holds every value given." },
  { name: "$elemMatch", info: "An element of the array matches all of it." },
  { name: "$size", info: "The array holds this many elements." },
  { name: "$bitsAllClear", info: "Every given bit is 0." },
  { name: "$bitsAllSet", info: "Every given bit is 1." },
  { name: "$bitsAnyClear", info: "Any given bit is 0." },
  { name: "$bitsAnySet", info: "Any given bit is 1." },
  { name: "$geoIntersects", info: "Intersects a GeoJSON geometry." },
  { name: "$geoWithin", info: "Within a bounding GeoJSON geometry." },
  { name: "$near", info: "Nearest to a point; needs a geospatial index." },
  {
    name: "$nearSphere",
    info: "Nearest to a point on a sphere; needs a geospatial index.",
  },
]

/** The operators of an update document: `{"$set": {…}}`. */
export const UPDATE_OPERATORS: ReadonlyArray<Word> = [
  { name: "$set", info: "Sets the value of a field." },
  { name: "$unset", info: "Removes a field." },
  { name: "$inc", info: "Increments a field by an amount." },
  { name: "$mul", info: "Multiplies a field by an amount." },
  { name: "$min", info: "Updates only if the value is less." },
  { name: "$max", info: "Updates only if the value is greater." },
  { name: "$rename", info: "Renames a field." },
  { name: "$currentDate", info: "Sets a field to the current date." },
  {
    name: "$setOnInsert",
    info: "Sets a field only when an upsert inserts.",
  },
  { name: "$push", info: "Adds an item to an array." },
  { name: "$addToSet", info: "Adds items not already in the array." },
  { name: "$pop", info: "Removes the first or last item of an array." },
  { name: "$pull", info: "Removes array elements that match a query." },
  { name: "$pullAll", info: "Removes every listed value from an array." },
  { name: "$bit", info: "Bitwise and, or, xor of an integer." },
]

/** The stages that write a collection: an aggregation holding one writes. */
const WRITING_STAGE = "Writes the results to a collection."

export const STAGES: ReadonlyArray<Word> = [
  { name: "$match", info: "Passes only the documents that match." },
  { name: "$project", info: "Reshapes each document: fields in or out." },
  { name: "$group", info: "Groups by _id and applies accumulators." },
  { name: "$sort", info: "Reorders the documents by a sort key." },
  { name: "$limit", info: "Passes the first n documents." },
  { name: "$skip", info: "Skips the first n documents." },
  { name: "$unwind", info: "One document per element of an array." },
  { name: "$lookup", info: "Left outer join with a collection." },
  { name: "$addFields", info: "Adds fields to documents." },
  { name: "$set", info: "Adds fields to documents ($addFields)." },
  { name: "$unset", info: "Removes fields from documents." },
  { name: "$count", info: "The number of documents at this stage." },
  { name: "$facet", info: "Several pipelines on the same input." },
  { name: "$bucket", info: "Groups into buckets by boundaries." },
  { name: "$bucketAuto", info: "Groups into n buckets, boundaries computed." },
  { name: "$sortByCount", info: "Groups by a value and counts each group." },
  { name: "$replaceRoot", info: "Replaces each document by an embedded one." },
  { name: "$replaceWith", info: "Replaces each document ($replaceRoot)." },
  { name: "$sample", info: "Randomly selects n documents." },
  { name: "$unionWith", info: "Appends the documents of another collection." },
  { name: "$graphLookup", info: "Recursive search on a collection." },
  { name: "$geoNear", info: "Documents ordered by distance to a point." },
  { name: "$densify", info: "Fills gaps in a sequence of values." },
  { name: "$fill", info: "Populates null and missing values." },
  { name: "$setWindowFields", info: "Window functions over sorted groups." },
  { name: "$redact", info: "Restricts content by the document's own data." },
  { name: "$documents", info: "Literal documents as the input." },
  { name: "$collStats", info: "Statistics of the collection." },
  { name: "$indexStats", info: "How each index of the collection is used." },
  { name: "$out", info: WRITING_STAGE },
  { name: "$merge", info: WRITING_STAGE },
]

/** The accumulators of a `$group` output field. */
export const ACCUMULATORS: ReadonlyArray<Word> = [
  { name: "$sum", info: "Sum of numeric values." },
  { name: "$avg", info: "Average of numeric values." },
  { name: "$min", info: "Lowest value." },
  { name: "$max", info: "Highest value." },
  { name: "$count", info: "Number of documents in the group." },
  { name: "$first", info: "Value for the first document." },
  { name: "$last", info: "Value for the last document." },
  { name: "$push", info: "Array of the values." },
  { name: "$addToSet", info: "Array of the unique values." },
  { name: "$mergeObjects", info: "One document combining the inputs." },
  { name: "$stdDevPop", info: "Population standard deviation." },
  { name: "$stdDevSamp", info: "Sample standard deviation." },
  { name: "$top", info: "Top element by a sort order." },
  { name: "$bottom", info: "Bottom element by a sort order." },
  { name: "$topN", info: "Top n elements by a sort order." },
  { name: "$bottomN", info: "Bottom n elements by a sort order." },
  { name: "$firstN", info: "First n values." },
  { name: "$lastN", info: "Last n values." },
  { name: "$maxN", info: "The n highest values." },
  { name: "$minN", info: "The n lowest values." },
  { name: "$median", info: "Approximate median." },
  { name: "$percentile", info: "Approximate percentiles." },
]

/** The verbosity values of `explain`. */
export const VERBOSITIES: ReadonlyArray<Word> = [
  { name: "queryPlanner", info: "The winning plan, without running it." },
  {
    name: "executionStats",
    info: "Runs the winning plan and reports its statistics.",
  },
  {
    name: "allPlansExecution",
    info: "Runs the candidate plans too; the default.",
  },
]
