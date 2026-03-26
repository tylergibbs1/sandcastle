// Example: using host capabilities
// The host must register capabilities before running this script

const api = globalThis.__sandcastle_modules["host:api"] || {};
const fs = globalThis.__sandcastle_fs;

// Use host API to get user data
if (api.getUser) {
    const user = api.getUser(42);
    console.log("Got user:", JSON.stringify(user));
}

// Read an input artifact
try {
    const data = fs.readFile("/data.csv");
    console.log("Read artifact:", data.length, "chars");
} catch(e) {
    console.log("No data.csv artifact mounted");
}

// Write an output artifact
fs.writeFile("/output/result.json", JSON.stringify({
    status: "completed",
    timestamp: new Date().toISOString()
}));

return { status: "ok" };
