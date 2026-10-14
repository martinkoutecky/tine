import { createMemo, type JSX } from "solid-js";
import { render, type Msg } from "./index";

type Tag = "a" | "em" | "strong" | "kbd" | "code";
const escape = (text: string) => text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
const unescape = (text: string) => text.replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&");

/** Whole catalog sentences with inline slots. Variables are escaped before
 * parsing, so graph text can never become markup. No HTML is injected. */
export function Rich(props: {
  message: Msg;
  slots?: Partial<Record<Tag, (children: JSX.Element) => JSX.Element>>;
}): JSX.Element {
  const content = createMemo(() => {
    const message = props.message;
    const args = message.args && Object.fromEntries(Object.entries(message.args)
      .map(([key, value]) => [key, typeof value === "string" ? escape(value) : value]));
    const text = render({ ...message, args });
    const tokens = text.split(/(<\/?(?:a|em|strong|kbd|code)>)/g);
    let index = 0;
    const children = (closing?: Tag): JSX.Element[] => {
      const out: JSX.Element[] = [];
      while (index < tokens.length) {
        const token = tokens[index++];
        if (closing && token === `</${closing}>`) break;
        const match = /^<(a|em|strong|kbd|code)>$/.exec(token);
        if (!match) { out.push(unescape(token)); continue; }
        const tag = match[1] as Tag;
        const body = children(tag);
        const slot = props.slots?.[tag];
        if (slot) out.push(slot(body));
        else switch (tag) {
          case "strong": out.push(<strong>{body}</strong>); break;
          case "em": out.push(<em>{body}</em>); break;
          case "kbd": out.push(<kbd>{body}</kbd>); break;
          case "code": out.push(<code>{body}</code>); break;
          case "a": out.push(body); break;
        }
      }
      return out;
    };
    return children();
  });
  return <>{content()}</>;
}
