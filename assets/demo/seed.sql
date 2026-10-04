-- Synthetic, deterministic SQLite data for the UI recordings.
-- Load into a NEW database: sqlite3 /tmp/oxyn-demo.sqlite < assets/demo/seed.sql
PRAGMA foreign_keys = ON;
BEGIN;
CREATE TABLE customers (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    country TEXT NOT NULL,
    plan TEXT NOT NULL CHECK (plan IN ('Studio', 'Team', 'Enterprise'))
);
CREATE TABLE products (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    category TEXT NOT NULL,
    price_eur REAL NOT NULL CHECK (price_eur >= 0)
);
CREATE TABLE orders (
    id INTEGER PRIMARY KEY,
    customer_id INTEGER NOT NULL REFERENCES customers(id),
    ordered_at TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('Paid', 'Pending', 'Refunded')),
    total_eur REAL NOT NULL CHECK (total_eur >= 0)
);
CREATE TABLE order_items (
    order_id INTEGER NOT NULL REFERENCES orders(id),
    product_id INTEGER NOT NULL REFERENCES products(id),
    quantity INTEGER NOT NULL CHECK (quantity > 0),
    unit_price_eur REAL NOT NULL,
    PRIMARY KEY (order_id, product_id)
);
INSERT INTO customers VALUES
    (1, 'Atelier North', 'France', 'Studio'),
    (2, 'Forma Studio', 'Germany', 'Team'),
    (3, 'Olive & Oak', 'Italy', 'Studio'),
    (4, 'Coast Collective', 'Portugal', 'Enterprise'),
    (5, 'Mono Works', 'Netherlands', 'Team'),
    (6, 'Lumen House', 'Denmark', 'Studio'),
    (7, 'Paper & Form', 'France', 'Team'),
    (8, 'Arc Editions', 'Spain', 'Enterprise');
INSERT INTO products VALUES
    (1, 'Desk lamp', 'Lighting', 89.00),
    (2, 'Linen notebook', 'Stationery', 24.00),
    (3, 'Ceramic mug', 'Home', 32.00),
    (4, 'Oak tray', 'Home', 56.00),
    (5, 'Art print', 'Prints', 45.00),
    (6, 'Canvas tote', 'Accessories', 28.00);
WITH RECURSIVE sequence(n) AS (
    SELECT 1 UNION ALL SELECT n + 1 FROM sequence WHERE n < 48
)
INSERT INTO orders
SELECT n, 1 + ((n - 1) % 8),
       printf('2026-%02d-%02d', 4 + ((n - 1) / 8), 2 + ((n * 3) % 26)),
       CASE WHEN n % 11 = 0 THEN 'Pending'
            WHEN n % 17 = 0 THEN 'Refunded' ELSE 'Paid' END,
       0
FROM sequence;
INSERT INTO order_items
SELECT o.id, p.id, 1 + (o.id % 4), p.price_eur
FROM orders o JOIN products p ON p.id = 1 + ((o.id - 1) % 6);
UPDATE orders SET total_eur = (
    SELECT SUM(quantity * unit_price_eur)
    FROM order_items WHERE order_id = orders.id
);
CREATE INDEX orders_customer_date ON orders(customer_id, ordered_at);
CREATE VIEW monthly_revenue AS
SELECT substr(ordered_at, 1, 7) AS month,
       COUNT(*) AS paid_orders,
       ROUND(SUM(total_eur), 2) AS revenue_eur
FROM orders WHERE status = 'Paid'
GROUP BY month;
COMMIT;
