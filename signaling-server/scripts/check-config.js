#!/usr/bin/env node
// Validates the server configuration without starting it.
//   npm run check-config                 → errors fail, production gaps are warnings
//   npm run check-config -- --production → production gaps are errors too
// Reads ./.env if present (same as the server). Prints names, never secret values.

require("dotenv").config({ quiet: true });
const { validateEnv } = require("../server");

const production = process.argv.includes("--production");
const { errors, warnings, secretsPresent } = validateEnv(process.env, { production });

console.log(`Talktual server configuration check (${production ? "production" : "development"} rules)`);
console.log("Provider keys present: " + Object.entries(secretsPresent).map(([k, v]) => `${k}=${v ? "yes" : "no"}`).join(", "));
errors.forEach((e) => console.log(`  ERROR    ${e}`));
warnings.forEach((w) => console.log(`  WARNING  ${w}`));
if (!errors.length && !warnings.length) console.log("  OK");
process.exit(errors.length ? 1 : 0);
