import type { JSX } from "solid-js";
import { ReferenceSection } from "./LinkedReferences";

export function UnlinkedReferences(props: { name: string }): JSX.Element {
  return <ReferenceSection name={props.name} unlinked />;
}
