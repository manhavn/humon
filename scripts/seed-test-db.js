// Test data for the live tests (HUMON_TEST_URI). Run with:
//   mongosh mongodb://127.0.0.1:27018 scripts/seed-test-db.js
const shop = db.getSiblingDB("shop");
shop.dropDatabase();
shop.users.insertMany(
  Array.from({ length: 137 }, (_, i) => ({
    name: "user" + i,
    age: 18 + (i % 50),
    tags: ["a", i % 2 ? "odd" : "even"],
    address: { city: i % 3 ? "Istanbul" : "Izmir", zip: NumberInt(34000 + i) },
    createdAt: new Date(Date.UTC(2024, 0, 1 + i)),
    balance: NumberDecimal("10.50"),
    big: NumberLong("9007199254740993"),
    uid: UUID(),
    ratio: i / 7,
  }))
);
shop.orders.insertOne({ total: 5, re: /ab+c/i });
shop.createView("adults", "users", [{ $match: { age: { $gte: 30 } } }]);
const logs = db.getSiblingDB("logs");
logs.dropDatabase();
logs.events.insertOne({ msg: "hi" });
print("seeded shop (" + shop.users.countDocuments() + " users) and logs");
