// 30 products of an outdoor gear shop.
import type { PrismaClient } from "@prisma/client";

const items: [string, string, string, number][] = [
  ["Ultralight Tent", "A two-person tent that weighs under one kilo.", "Camping", 349],
  ["Family Tent", "Room for six, with two bedrooms and a porch.", "Camping", 499],
  ["Sleeping Bag", "Keeps you warm down to minus five degrees.", "Camping", 189],
  ["Sleeping Pad", "Self-inflating pad that packs as small as a bottle.", "Camping", 79],
  ["Camp Pillow", "Inflatable pillow with a soft brushed cover.", "Camping", 25],
  ["Hammock", "Parachute nylon hammock with tree straps included.", "Camping", 59],
  ["Camp Chair", "Folding chair that sets up in ten seconds.", "Camping", 69],
  ["Camp Table", "Aluminium table for four, rolls up for transport.", "Camping", 89],
  ["Gas Stove", "Boils a litre of water in three minutes.", "Kitchen", 45],
  ["Cook Set", "Two pots, a pan and a lid that nest together.", "Kitchen", 65],
  ["Water Filter", "Removes bacteria from streams and lakes.", "Kitchen", 39],
  ["Insulated Bottle", "Keeps drinks hot for twelve hours.", "Kitchen", 29],
  ["Camp Mug", "Stainless steel mug with a folding handle.", "Kitchen", 15],
  ["Spork", "Spoon, fork and knife in one titanium tool.", "Kitchen", 12],
  ["Cooler Box", "Keeps food cold for three days.", "Kitchen", 129],
  ["Hiking Boots", "Waterproof leather boots with a grippy sole.", "Footwear", 179],
  ["Trail Runners", "Light shoes for fast hikes on rough trails.", "Footwear", 139],
  ["Camp Sandals", "Easy sandals to rest your feet at camp.", "Footwear", 49],
  ["Wool Socks", "Merino socks that stay fresh for days.", "Footwear", 19],
  ["Gaiters", "Keep snow and mud out of your boots.", "Footwear", 35],
  ["Rain Jacket", "Breathable shell for wet and windy days.", "Clothing", 199],
  ["Down Jacket", "Warm, packable jacket for cold evenings.", "Clothing", 249],
  ["Fleece", "Soft mid layer for chilly mornings.", "Clothing", 79],
  ["Hiking Pants", "Stretch pants that turn into shorts.", "Clothing", 89],
  ["Sun Hat", "Wide brim hat with UV protection.", "Clothing", 29],
  ["Backpack 40L", "A comfortable pack for weekend trips.", "Packs", 159],
  ["Daypack", "Small pack for day hikes with a bladder pocket.", "Packs", 79],
  ["Headlamp", "Bright light with a red night mode.", "Lighting", 39],
  ["Lantern", "Rechargeable lantern that lights up the whole tent.", "Lighting", 49],
  ["Trekking Poles", "Carbon poles that fold into three parts.", "Accessories", 119],
];

export async function seed(prisma: PrismaClient) {
  await prisma.product.createMany({
    data: items.map(([name, description, category, price]) => ({ name, description, category, price })),
  });
  return items.length;
}
