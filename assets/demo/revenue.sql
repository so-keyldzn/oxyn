SELECT c.country,
       COUNT(*) AS paid_orders,
       ROUND(SUM(o.total_eur), 2) AS revenue_eur,
       ROUND(AVG(o.total_eur), 2) AS average_order_eur
FROM orders AS o JOIN customers AS c ON c.id = o.customer_id
WHERE o.status = 'Paid'
GROUP BY c.country
ORDER BY revenue_eur DESC;
