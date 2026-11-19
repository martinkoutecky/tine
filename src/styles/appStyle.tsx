import { render } from "solid-js/web";
import { ThreadJoinMasks } from "../components/ThreadJoinMasks";
import { documentOf, mainWindow } from "../windowRealm";
import "./app.css";

render(() => <ThreadJoinMasks />, documentOf(mainWindow).body);
