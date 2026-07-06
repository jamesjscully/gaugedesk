/* @refresh reload */
import { render } from "solid-js/web";
import { MobileApp } from "./MobileApp";
import "@gaugewright/workbench-ui/styles.css";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");
render(() => <MobileApp />, root);
