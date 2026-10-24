import { StreamLanguage } from "@codemirror/language"
import type { StreamParser } from "@codemirror/language"
import type { Extension } from "@codemirror/state"

/**
 * How a console reads a Redis command line, for highlighting only.
 *
 * The grammar is the driver's (`oxyn_driver_redis::argv`, itself
 * `redis-cli`'s `sdssplitargs`): blanks separate arguments, double quotes take
 * escapes, single quotes take only `\'`. The first argument is the command.
 * No command list is written here: which names exist is the server's to say
 * (ADR-0061), and a name highlighted as a command proves nothing about it.
 */
interface RedisLineState {
  /** No argument read yet on this command: the next one is its name. */
  expectCommand: boolean
  /** Inside a quoted argument that a line break did not close. */
  quote: '"' | "'" | null
}

const BLANK = /[ \t\r\f\v]/
const NUMBER = /^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?(?=[ \t]|$)/

function quoted(
  stream: Parameters<StreamParser<RedisLineState>["token"]>[0],
  state: RedisLineState,
  quote: '"' | "'"
): string {
  while (!stream.eol()) {
    const next = stream.next()
    if (next === "\\") {
      // In double quotes every escape takes the next character; in single
      // quotes only `\'` is one, which also takes it.
      if (quote === '"' || stream.peek() === "'") stream.next()
    } else if (next === quote) {
      state.quote = null
      return "string"
    }
  }
  state.quote = quote
  return "string"
}

export const redisCommandParser: StreamParser<RedisLineState> = {
  name: "redis",
  startState: () => ({ expectCommand: true, quote: null }),
  copyState: (state) => ({ ...state }),
  token(stream, state) {
    if (state.quote) return quoted(stream, state, state.quote)
    if (stream.sol()) state.expectCommand = true
    if (stream.eatSpace()) return null
    const command = state.expectCommand
    state.expectCommand = false
    const start = stream.peek()
    if (start === '"' || start === "'") {
      stream.next()
      return quoted(stream, state, start)
    }
    if (!command && stream.match(NUMBER)) return "number"
    while (!stream.eol()) {
      const next = stream.peek()
      if (
        next === undefined ||
        BLANK.test(next) ||
        next === '"' ||
        next === "'"
      )
        break
      stream.next()
    }
    return command ? "keyword" : null
  },
}

/** The editor mode of a console that sends Redis commands. */
export function redisEditorMode(): Extension {
  return StreamLanguage.define(redisCommandParser)
}
