const express = require('express');
const app = express();

app.use(express.json());

app.post('/', (req, res) => {
  const { name } = req.body;

  if (!name) {
    return res.status(400).json({ error: 'Name is required' });
  }

  res.json({ message: `Hello, ${name}` });
});

app.listen(3000, () => {
  console.log('Greeter server running on port 3000');
});
