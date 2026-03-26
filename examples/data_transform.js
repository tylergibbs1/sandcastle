// Example: data transformation agent task
const input = globalThis.__sandcastle_input;

// Parse CSV-like data
const rows = input.data.split('\n').filter(r => r.trim());
const headers = rows[0].split(',').map(h => h.trim());
const records = rows.slice(1).map(row => {
    const values = row.split(',').map(v => v.trim());
    const obj = {};
    headers.forEach((h, i) => { obj[h] = values[i]; });
    return obj;
});

// Filter and transform
const result = records
    .filter(r => parseFloat(r.amount) > 100)
    .map(r => ({
        name: r.name.toUpperCase(),
        amount: parseFloat(r.amount),
        category: r.category || 'uncategorized'
    }))
    .sort((a, b) => b.amount - a.amount);

console.log(`Processed ${records.length} records, ${result.length} matched filter`);

return { processed: result, total: result.reduce((s, r) => s + r.amount, 0) };
