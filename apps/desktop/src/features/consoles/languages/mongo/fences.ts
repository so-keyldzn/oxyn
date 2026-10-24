/**
 * The fence labels that name a MongoDB console command in an answer: the
 * console holds one database command as Extended JSON (ADR-0063). Not
 * `javascript`: a shell script is not a command the console sends.
 */
export const MONGO_FENCES: ReadonlyArray<string> = ["json", "mongo", "mongodb"]
