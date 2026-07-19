import "./style.css";
import { App } from "./app";

const root = document.getElementById("app");
if (root) {
  const app = new App(root);
  app.render();
}