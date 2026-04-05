const express = require('express');
const app = express();

app.use(express.json());

function parseItem(item, index) {
  if (!item || typeof item !== 'object') {
    throw new Error(`Item ${index + 1} is invalid`);
  }

  const { sku, name, quantity, price } = item;

  if (!sku || !name) {
    throw new Error(`Item ${index + 1} must include sku and name`);
  }

  if (!Number.isInteger(quantity) || quantity <= 0) {
    throw new Error(`Item ${index + 1} must have a positive integer quantity`);
  }

  if (typeof price !== 'number' || price < 0) {
    throw new Error(`Item ${index + 1} must have a valid price`);
  }

  const lineTotal = Math.round(price * quantity * 100) / 100;

  return {
    sku,
    name,
    quantity,
    price,
    lineTotal,
  };
}

function calculateDiscount(subtotal, couponCode, itemCount) {
  if (couponCode === 'WELCOME10') {
    return Math.round(subtotal * 0.1 * 100) / 100;
  }

  if (couponCode === 'BULK15' && itemCount >= 10) {
    return Math.round(subtotal * 0.15 * 100) / 100;
  }

  return 0;
}

function calculateShipping(subtotal, country, expedited) {
  if (subtotal >= 100) {
    return 0;
  }

  const baseShipping = country === 'US' ? 5.99 : 12.99;
  return expedited ? baseShipping + 8 : baseShipping;
}

app.post('/', (req, res) => {
  const {
    orderId,
    customer,
    items,
    shippingCountry = 'US',
    couponCode,
    expedited = false,
  } = req.body || {};

  if (!orderId || !customer || !customer.name || !Array.isArray(items) || items.length === 0) {
    return res.status(400).json({
      error: 'orderId, customer.name and a non-empty items array are required',
    });
  }

  try {
    const normalizedItems = items.map(parseItem);
    const subtotal = normalizedItems.reduce((sum, item) => sum + item.lineTotal, 0);
    const discount = calculateDiscount(subtotal, couponCode, normalizedItems.length);
    const shipping = calculateShipping(subtotal - discount, shippingCountry, expedited);
    const tax = Math.round((subtotal - discount + shipping) * 0.08 * 100) / 100;
    const total = Math.round((subtotal - discount + shipping + tax) * 100) / 100;

    res.json({
      orderId,
      customer: {
        name: customer.name,
        email: customer.email || null,
      },
      pricing: {
        subtotal,
        discount,
        shipping,
        tax,
        total,
      },
      fulfillment: {
        shippingCountry,
        expedited,
        couponApplied: discount > 0 ? couponCode || null : null,
      },
      items: normalizedItems,
      processedAt: new Date().toISOString(),
    });
  } catch (error) {
    return res.status(400).json({ error: error.message });
  }
});

app.listen(3000, () => {
  console.log('Order processing server running on port 3000');
});
