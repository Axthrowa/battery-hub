import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { SettingsProvider, applyStoredTheme } from "./context/SettingsContext";
import "./i18n";
import "./styles.css";

applyStoredTheme();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <SettingsProvider>
      <App />
    </SettingsProvider>
  </React.StrictMode>,
);
