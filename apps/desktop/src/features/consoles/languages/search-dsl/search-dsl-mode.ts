import { StreamLanguage } from "@codemirror/language"
import type { StreamParser } from "@codemirror/language"
import type { Extension } from "@codemirror/state"

/**
 * How a console reads Elasticsearch and OpenSearch requests, for
 * highlighting only.
 *
 * The grammar is the driver's (`oxyn_driver_elasticsearch::request`): a line
 * that starts with an HTTP method opens a request — the method, then its API
 * path —, and the lines after it are its JSON body. In the path, the
 * segments that name the API (`_search`, `_cat`) stand out from the index
 * names. No endpoint list is written here: which APIs exist is the server's
 * to say, and a segment highlighted as an API proves nothing about it.
 */
interface SearchDslState {
  /** On the request line, after the method: the rest is the path. */
  inPath: boolean
  /** Inside a JSON string that a line break did not close. */
  inString: boolean
}

const METHOD = /^(?:GET|POST|PUT|PATCH|DELETE|HEAD)(?=\s|$)/i
const NUMBER = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/

type Stream = Parameters<StreamParser<SearchDslState>["token"]>[0]

/** The rest of a JSON string, its closing quote included. */
function jsonString(stream: Stream, state: SearchDslState): string {
  while (!stream.eol()) {
    const next = stream.next()
    if (next === "\\") stream.next()
    else if (next === '"') {
      state.inString = false
      // A string followed by `:` is an object's key.
      return stream.match(/^\s*:/, false) ? "propertyName" : "string"
    }
  }
  state.inString = true
  return "string"
}

/** One segment of the API path: `_search` is an API, `logs-*` a target. */
function pathSegment(stream: Stream): string {
  if (stream.eat("/")) return "punctuation"
  if (stream.eat("?")) {
    stream.skipToEnd()
    return "attributeName"
  }
  const api = stream.peek() === "_"
  stream.eatWhile(/[^/?\s]/)
  return api ? "keyword" : "url"
}

export const searchDslParser: StreamParser<SearchDslState> = {
  name: "search-dsl",
  startState: () => ({ inPath: false, inString: false }),
  copyState: (state) => ({ ...state }),
  token(stream, state) {
    if (state.inString) return jsonString(stream, state)
    if (stream.sol()) {
      state.inPath = false
      if (stream.match(/^\s*(?:#|\/\/).*/)) return "comment"
      stream.eatSpace()
      if (stream.match(METHOD)) {
        state.inPath = true
        return "keyword"
      }
    }
    if (stream.eatSpace()) return null
    if (state.inPath) return pathSegment(stream)
    if (stream.eat('"')) return jsonString(stream, state)
    if (stream.match(NUMBER)) return "number"
    if (stream.match(/^(?:true|false)\b/)) return "bool"
    if (stream.match(/^null\b/)) return "null"
    const next = stream.next()
    return next !== undefined && "{}[],:".includes(next) ? "punctuation" : null
  },
}

/** The editor mode of a console that sends Elasticsearch requests. */
export function searchDslEditorMode(): Extension {
  return StreamLanguage.define(searchDslParser)
}
