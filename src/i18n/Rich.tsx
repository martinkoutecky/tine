import { createMemo, type JSX } from "solid-js";
import { render, type Msg } from "./index";

const escape = (text: string) => text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
const unescape = (text: string) => text.replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&");

const TAG = /(<\/?[a-z][a-z0-9-]*>)/g;

/** A catalog sentence's text as `Rich` shows it, without its inline tags: for
 * search and other plain-text uses (GH #706: "<em>" showed in Settings search). */
export function richPlain(text: string): string {
  return text.split(TAG).filter((token, index) => index % 2 === 0).join("");
}

/** Whole catalog sentences with inline slots. Variables are escaped before
 * parsing, so graph text can never become markup. No HTML is injected. */
export function Rich(props: {
  message: Msg;
  slots?: Record<string, (children: JSX.Element) => JSX.Element>;
}): JSX.Element {
  const content = createMemo(() => {
    const message = props.message;
    const args = message.args && Object.fromEntries(Object.entries(message.args)
      .map(([key, value]) => [key, typeof value === "string" ? escape(value) : value]));
    const text = render({ ...message, args });
    const tokens = text.split(TAG);
    const children = (start: number, end: number): JSX.Element[] => {
      const out: JSX.Element[] = [];
      for (let index = start; index < end; index++) {
        const token = tokens[index];
        const match = /^<([a-z][a-z0-9-]*)>$/.exec(token);
        if (!match) { out.push(unescape(token)); continue; }
        const tag = match[1];
        let closing = index + 1;
        let depth = 1;
        for (; closing < end; closing++) {
          if (tokens[closing] === token) depth++;
          if (tokens[closing] === `</${tag}>` && --depth === 0) break;
        }
        if (closing === end) { out.push(unescape(token)); continue; }
        // Unknown wrappers remain literal in their entirety, even if they
        // contain known tags. Only own properties can name a caller's slot.
        const slot = props.slots && Object.hasOwn(props.slots, tag) ? props.slots[tag] : undefined;
        const known = ["a", "em", "strong", "kbd", "code"].includes(tag);
        if (!slot && !known) {
          out.push(unescape(tokens.slice(index, closing + 1).join("")));
          index = closing;
          continue;
        }
        const body = children(index + 1, closing);
        if (slot) out.push(slot(body));
        else switch (tag) {
          case "strong": out.push(<strong>{body}</strong>); break;
          case "em": out.push(<em>{body}</em>); break;
          case "kbd": out.push(<kbd>{body}</kbd>); break;
          case "code": out.push(<code>{body}</code>); break;
          case "a": out.push(body); break;
        }
        index = closing;
      }
      return out;
    };
    return children(0, tokens.length);
  });
  return <>{content()}</>;
}
