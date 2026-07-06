/* @refresh reload */
import { render } from "solid-js/web";
import { consumeCallbackToken } from "@gaugewright/control-plane-client";
import { EnterpriseControlPlane } from "@gaugewright/enterprise-client";
import "@gaugewright/workbench-ui/styles.css";
import { AdminConsole } from "./AdminConsole";

consumeCallbackToken();
const api = new EnterpriseControlPlane();
const close = () => {
    if (window.history.length > 1) window.history.back();
};

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");
render(() => <AdminConsole api={api} onClose={close} />, root);
