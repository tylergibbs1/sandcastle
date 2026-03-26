// Simple hello world example
console.log("Hello from SandCastle!");

const input = globalThis.__sandcastle_input;
console.log("Input:", JSON.stringify(input));

// Return a result
return { greeting: "Hello, World!", input: input };
