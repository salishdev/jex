#!/usr/bin/env python3
"""Generate a deterministic, deeply nested JSON fixture for jex."""

from __future__ import annotations

import argparse
import json
import random
from datetime import UTC, datetime, timedelta
from pathlib import Path


DEFAULT_OUTPUT = Path(__file__).with_name("large-demo.json")
BASE_TIME = datetime(2025, 1, 15, 12, 0, tzinfo=UTC)
FIRST_NAMES = [
    "Ada",
    "Linus",
    "Grace",
    "Edsger",
    "Margaret",
    "Ken",
    "Barbara",
    "Donald",
    "Radia",
    "Guido",
    "Yukihiro",
    "Hedy",
]
LAST_NAMES = [
    "Lovelace",
    "Torvalds",
    "Hopper",
    "Dijkstra",
    "Hamilton",
    "Thompson",
    "Liskov",
    "Knuth",
    "Perlman",
    "Rossum",
    "Matsumoto",
    "Lamarr",
]
CITIES = [
    ("Seattle", "WA", "US", "98101"),
    ("Portland", "OR", "US", "97205"),
    ("Vancouver", "BC", "CA", "V6B 1A1"),
    ("Austin", "TX", "US", "78701"),
    ("Berlin", "BE", "DE", "10115"),
    ("Tokyo", "13", "JP", "100-0001"),
    ("São Paulo", "SP", "BR", "01000-000"),
    ("München", "BY", "DE", "80331"),
]
PRODUCT_WORDS = [
    "Aurora",
    "Cedar",
    "Drift",
    "Ember",
    "Fjord",
    "Harbor",
    "Juniper",
    "Kestrel",
    "Lumen",
    "Nimbus",
    "Orca",
    "Summit",
]


def iso(offset: timedelta) -> str:
    return (BASE_TIME + offset).isoformat().replace("+00:00", "Z")


def address(index: int, kind: str) -> dict[str, object]:
    city, region, country, postal_code = CITIES[index % len(CITIES)]
    return {
        "id": f"addr_{index:05d}_{kind}",
        "kind": kind,
        "recipient": f"{FIRST_NAMES[index % len(FIRST_NAMES)]} {LAST_NAMES[index % len(LAST_NAMES)]}",
        "lines": [f"{100 + index} Example Avenue", None if index % 5 else "Suite 200"],
        "locality": {
            "city": city,
            "administrative_area": region,
            "postal_code": postal_code,
            "country_code": country,
        },
        "delivery": {
            "instructions": "Leave with concierge" if index % 4 == 0 else None,
            "coordinates": {
                "latitude": round(47.60 - (index % 20) * 0.031, 6),
                "longitude": round(-122.33 + (index % 20) * 0.047, 6),
            },
            "validated": index % 7 != 0,
        },
    }


def make_catalog() -> tuple[dict[str, object], list[dict[str, object]]]:
    categories = []
    products = []
    product_index = 0
    for category_index in range(10):
        category_id = f"cat_{category_index + 1:02d}"
        child_categories = []
        for child_index in range(3):
            child_id = f"{category_id}_{child_index + 1}"
            child_categories.append(
                {
                    "id": child_id,
                    "name": f"{PRODUCT_WORDS[category_index]} Collection {child_index + 1}",
                    "filters": {
                        "colors": ["midnight", "sand", "forest", "海色"],
                        "sizes": ["XS", "S", "M", "L", "XL"],
                        "price": {"minimum": 1000, "maximum": 50000, "currency": "USD"},
                    },
                }
            )
        categories.append(
            {
                "id": category_id,
                "slug": f"collection-{category_index + 1}",
                "name": f"{PRODUCT_WORDS[category_index]} Goods",
                "children": child_categories,
            }
        )

        for item_index in range(12):
            product_index += 1
            variants = []
            for variant_index, color in enumerate(["midnight", "sand", "forest"]):
                variants.append(
                    {
                        "id": f"var_{product_index:04d}_{variant_index + 1}",
                        "sku": f"SKU-{product_index:04d}-{variant_index + 1}",
                        "attributes": {
                            "color": color,
                            "size": ["S", "M", "L"][variant_index],
                            "material": {
                                "primary": "recycled nylon" if product_index % 2 else "organic cotton",
                                "composition": [
                                    {"fiber": "base", "percentage": 85},
                                    {"fiber": "reinforcement", "percentage": 15},
                                ],
                            },
                        },
                        "price": {
                            "amount_minor": 1500 + product_index * 37 + variant_index * 250,
                            "currency": "USD",
                            "compare_at_minor": None if product_index % 4 else 3500 + product_index * 37,
                        },
                        "inventory": {
                            region: {
                                "available": (product_index * (variant_index + 3)) % 137,
                                "reserved": (product_index + variant_index) % 11,
                                "incoming": [] if product_index % 3 else [{"quantity": 48, "eta": "2025-02-01"}],
                            }
                            for region in ["us-west", "us-east", "eu-central"]
                        },
                    }
                )
            products.append(
                {
                    "id": f"prod_{product_index:04d}",
                    "category_id": category_id,
                    "name": f"{PRODUCT_WORDS[product_index % len(PRODUCT_WORDS)]} Item {product_index}",
                    "description": "A durable everyday item designed for travel, work, and unexpected weather. ☂",
                    "status": "archived" if product_index % 29 == 0 else "active",
                    "tags": ["featured"] if product_index % 7 == 0 else [],
                    "variants": variants,
                    "media": {
                        "primary": f"https://cdn.example.test/products/{product_index}/primary.webp",
                        "gallery": [
                            {
                                "url": f"https://cdn.example.test/products/{product_index}/{photo}.webp",
                                "width": 1600,
                                "height": 1200,
                                "alt": f"Product {product_index}, view {photo}",
                            }
                            for photo in range(1, 4)
                        ],
                    },
                }
            )
    return {"categories": categories, "products": products}, products


def make_order(
    customer_index: int,
    order_index: int,
    orders_per_customer: int,
    products: list[dict[str, object]],
    rng: random.Random,
) -> dict[str, object]:
    absolute_index = customer_index * orders_per_customer + order_index
    item_count = 1 + absolute_index % 4
    line_items = []
    subtotal = 0
    for line_index in range(item_count):
        product = products[(absolute_index * 5 + line_index * 11) % len(products)]
        variant = product["variants"][(absolute_index + line_index) % 3]
        quantity = 1 + (absolute_index + line_index) % 3
        unit_price = variant["price"]["amount_minor"]
        line_total = quantity * unit_price
        subtotal += line_total
        line_items.append(
            {
                "line_id": f"line_{absolute_index:06d}_{line_index + 1}",
                "product": {
                    "id": product["id"],
                    "name": product["name"],
                    "variant": {"id": variant["id"], "sku": variant["sku"], **variant["attributes"]},
                },
                "quantity": quantity,
                "pricing": {
                    "unit_amount_minor": unit_price,
                    "discounts": []
                    if absolute_index % 5
                    else [{"code": "WELCOME10", "amount_minor": unit_price // 10}],
                    "tax_lines": [
                        {"name": "State tax", "rate": 0.065, "amount_minor": round(line_total * 0.065)},
                        {"name": "Local tax", "rate": 0.01, "amount_minor": round(line_total * 0.01)},
                    ],
                    "line_total_minor": line_total,
                    "currency": "USD",
                },
                "fulfillment": {
                    "warehouse": ["sea-01", "pdx-02", "fra-01"][absolute_index % 3],
                    "lot_numbers": [f"LOT-{absolute_index:06d}-{line_index + 1}"],
                    "serial_numbers": [] if quantity > 1 else [f"SN-{rng.randrange(10**10):010d}"],
                },
            }
        )

    tax = round(subtotal * 0.075)
    shipping = 0 if subtotal >= 7500 else 799
    total = subtotal + tax + shipping
    created_offset = timedelta(days=-(absolute_index % 365), hours=-(absolute_index % 24))
    status = ["delivered", "shipped", "processing", "cancelled"][absolute_index % 4]
    return {
        "id": f"ord_{absolute_index:06d}",
        "number": f"JEX-{100000 + absolute_index}",
        "created_at": iso(created_offset),
        "updated_at": iso(created_offset + timedelta(hours=7)),
        "status": status,
        "channel": ["web", "mobile", "marketplace"][absolute_index % 3],
        "line_items": line_items,
        "totals": {
            "subtotal_minor": subtotal,
            "shipping_minor": shipping,
            "tax_minor": tax,
            "discount_minor": 0,
            "grand_total_minor": total,
            "currency": "USD",
        },
        "payment": {
            "method": {
                "type": ["card", "wallet", "bank_transfer"][absolute_index % 3],
                "brand": "visa" if absolute_index % 3 == 0 else None,
                "last_four": f"{1000 + absolute_index % 9000:04d}",
            },
            "transactions": [
                {
                    "id": f"txn_{absolute_index:06d}",
                    "kind": "capture",
                    "status": "succeeded" if status != "cancelled" else "voided",
                    "amount_minor": total,
                    "processed_at": iso(created_offset + timedelta(minutes=3)),
                    "risk": {
                        "score": (absolute_index * 17) % 100,
                        "decision": "review" if absolute_index % 23 == 0 else "accept",
                        "signals": ["new_device", "address_mismatch"] if absolute_index % 23 == 0 else [],
                    },
                }
            ],
        },
        "shipment": None
        if status in ["processing", "cancelled"]
        else {
            "carrier": "Example Parcel",
            "service": "ground",
            "tracking_number": f"EX{absolute_index:014d}",
            "events": [
                {
                    "code": code,
                    "occurred_at": iso(created_offset + timedelta(days=day)),
                    "location": {"city": CITIES[(absolute_index + day) % len(CITIES)][0], "country": "US"},
                }
                for day, code in [(1, "label_created"), (2, "in_transit"), (4, "delivered")]
            ],
        },
        "notes": "Customer requested gift wrapping 🎁" if absolute_index % 13 == 0 else None,
    }


def make_customers(
    count: int,
    orders_per_customer: int,
    products: list[dict[str, object]],
    rng: random.Random,
) -> list[dict[str, object]]:
    customers = []
    for index in range(count):
        first = FIRST_NAMES[index % len(FIRST_NAMES)]
        last = LAST_NAMES[(index * 5) % len(LAST_NAMES)]
        customers.append(
            {
                "id": f"cus_{index + 1:05d}",
                "external_ids": {
                    "crm": f"CRM-{400000 + index}",
                    "legacy": None if index % 6 else f"LEGACY-{index:06d}",
                },
                "profile": {
                    "name": {"given": first, "family": last, "display": f"{first} {last}"},
                    "email": f"{first.lower()}.{last.lower()}.{index}@example.test",
                    "phone": f"+1-206-555-{index % 10000:04d}",
                    "birth_date": None if index % 4 else f"19{70 + index % 28:02d}-{1 + index % 12:02d}-15",
                    "locale": ["en-US", "en-CA", "de-DE", "ja-JP", "pt-BR"][index % 5],
                    "time_zone": ["America/Los_Angeles", "Europe/Berlin", "Asia/Tokyo"][index % 3],
                },
                "addresses": [address(index * 2, "shipping"), address(index * 2 + 1, "billing")],
                "preferences": {
                    "marketing": {
                        "email": index % 3 != 0,
                        "sms": index % 5 == 0,
                        "topics": ["new_arrivals", "field_notes"] if index % 2 else [],
                    },
                    "accessibility": {
                        "high_contrast": index % 19 == 0,
                        "reduced_motion": index % 11 == 0,
                        "screen_reader": None,
                    },
                },
                "loyalty": {
                    "tier": ["trail", "summit", "expedition"][index % 3],
                    "points": (index * 137) % 10000,
                    "history": [
                        {
                            "event": "earned" if event % 2 == 0 else "redeemed",
                            "points": 25 + event * 10,
                            "at": iso(timedelta(days=-(index + event * 10))),
                        }
                        for event in range(4)
                    ],
                },
                "orders": [
                    make_order(index, order, orders_per_customer, products, rng)
                    for order in range(orders_per_customer)
                ],
                "segments": [f"cohort-{2020 + index % 6}", "repeat-buyer"] if index % 3 == 0 else ["standard"],
                "deleted_at": None,
            }
        )
    return customers


def make_infrastructure() -> dict[str, object]:
    regions = []
    for region_index, region_name in enumerate(["us-west", "us-east", "eu-central", "ap-northeast"]):
        zones = []
        for zone_index in range(3):
            clusters = []
            for cluster_index in range(3):
                services = []
                for service_index, service_name in enumerate(["api", "worker", "search", "events"]):
                    services.append(
                        {
                            "name": service_name,
                            "deployment": {
                                "version": f"2025.01.{10 + (region_index + zone_index + cluster_index) % 6}",
                                "replicas": 2 + service_index,
                                "strategy": {"type": "rolling", "max_unavailable": 1, "max_surge": 2},
                            },
                            "resources": {
                                "requests": {"cpu_millicores": 250 * (service_index + 1), "memory_mebibytes": 256 * (service_index + 1)},
                                "limits": {"cpu_millicores": 1000 * (service_index + 1), "memory_mebibytes": 1024 * (service_index + 1)},
                            },
                            "health": {
                                "status": "degraded" if (region_index, zone_index, service_index) == (2, 1, 3) else "healthy",
                                "checks": [
                                    {"name": "readiness", "passing": True, "latency_ms": 4 + service_index},
                                    {"name": "dependencies", "passing": service_name != "events" or region_index != 2, "latency_ms": 18 + region_index},
                                ],
                            },
                        }
                    )
                clusters.append({"id": f"{region_name}-{zone_index + 1}-c{cluster_index + 1}", "services": services})
            zones.append({"name": f"{region_name}-{chr(97 + zone_index)}", "clusters": clusters})
        regions.append({"name": region_name, "primary": region_index == 0, "zones": zones})
    return {"regions": regions}


def make_document(customer_count: int, orders_per_customer: int, seed: int) -> dict[str, object]:
    rng = random.Random(seed)
    catalog, products = make_catalog()
    return {
        "fixture": {
            "name": "jex large nested demo",
            "schema_version": "1.0.0",
            "generated_at": "2025-01-15T12:00:00Z",
            "generator": "examples/generate_large_demo.py",
            "parameters": {"customers": customer_count, "orders_per_customer": orders_per_customer, "seed": seed},
            "purpose": ["tree navigation", "search", "JSON Pointer jumps", "large value rendering"],
        },
        "organization": {
            "id": "org_salish_outfitters",
            "name": "Salish Outfitters",
            "legal": {
                "registered_name": "Salish Outfitters Example Corporation",
                "jurisdictions": [
                    {"country": "US", "region": "WA", "registration": "EXAMPLE-001"},
                    {"country": "CA", "region": "BC", "registration": "EXAMPLE-002"},
                ],
            },
            "settings": {
                "localization": {
                    "default_locale": "en-US",
                    "supported_locales": ["en-US", "en-CA", "de-DE", "ja-JP", "pt-BR"],
                    "translations": {"welcome": {"en-US": "Welcome", "de-DE": "Willkommen", "ja-JP": "ようこそ"}},
                },
                "security": {
                    "authentication": {
                        "providers": [
                            {"type": "oidc", "issuer": "https://identity.example.test", "scopes": ["openid", "profile", "email"]},
                            {"type": "passkey", "relying_party": "shop.example.test", "enabled": True},
                        ],
                        "session": {"idle_timeout_seconds": 3600, "absolute_timeout_seconds": 86400},
                    },
                    "data_retention": {"orders_days": 2555, "events_days": 90, "anonymous_sessions_days": 30},
                },
                "feature_flags": {
                    "new_checkout": {"enabled": True, "rollout_percentage": 35, "allowlist": ["cus_00001", "cus_00042"]},
                    "recommendations_v2": {"enabled": False, "rollout_percentage": 0, "allowlist": []},
                },
            },
        },
        "catalog": catalog,
        "customers": make_customers(customer_count, orders_per_customer, products, rng),
        "infrastructure": make_infrastructure(),
        "analytics": {
            "daily": [
                {
                    "date": (BASE_TIME.date() - timedelta(days=day)).isoformat(),
                    "traffic": {
                        "sessions": 12000 + day * 137,
                        "unique_visitors": 8400 + day * 83,
                        "sources": {"direct": 0.31, "search": 0.42, "social": 0.17, "referral": 0.10},
                    },
                    "commerce": {
                        "orders": 430 + day % 37,
                        "gross_revenue_minor": 4800000 + day * 17003,
                        "refunds_minor": 75000 + day * 313,
                        "conversion_rate": round(0.031 + (day % 5) * 0.001, 4),
                    },
                }
                for day in range(90)
            ]
        },
        "edge_cases": {
            "empty_object": {},
            "empty_array": [],
            "null_value": None,
            "unicode": "Crème brûlée — 東京 — مرحبًا — 🦀",
            "escaped_string": "first line\nsecond line\t\"quoted\" and a backslash: \\",
            "numbers": {"zero": 0, "negative": -42, "fraction": 0.000001, "large_integer": 9007199254740991, "scientific": 6.022e23},
            "pointer/segment": {"tilde~segment": {"both~/together": "jump to /edge_cases/pointer~1segment/tilde~0segment/both~0~1together"}},
            "deeply": {
                "nested": {
                    "object": {
                        "chain": {
                            "continues": {
                                "through": {
                                    "many": {
                                        "levels": {
                                            "to": {"a": {"leaf": "depth-12"}}
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            },
        },
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--customers", type=int, default=120)
    parser.add_argument("--orders-per-customer", type=int, default=6)
    parser.add_argument("--seed", type=int, default=20250115)
    args = parser.parse_args()

    document = make_document(args.customers, args.orders_per_customer, args.seed)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", encoding="utf-8") as output:
        json.dump(document, output, ensure_ascii=False, indent=2)
        output.write("\n")


if __name__ == "__main__":
    main()
