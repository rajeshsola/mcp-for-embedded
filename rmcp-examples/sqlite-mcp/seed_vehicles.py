"""
Creates vehicles.db with four tables of sample vehicle data:
  vehicles    – fleet of cars/trucks
  parameters  – OBD-II / sensor parameter catalogue
  readings    – time-series sensor readings per vehicle
  maintenance – service history per vehicle
"""

import sqlite3, os, random
from datetime import datetime, timedelta

DB = os.path.join(os.path.dirname(__file__), "vehicles.db")

SCHEMA = """
CREATE TABLE IF NOT EXISTS vehicles (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    vin         TEXT    NOT NULL UNIQUE,
    make        TEXT    NOT NULL,
    model       TEXT    NOT NULL,
    year        INTEGER NOT NULL,
    color       TEXT,
    fuel_type   TEXT    NOT NULL DEFAULT 'gasoline',  -- gasoline | diesel | electric | hybrid
    odometer_km REAL    NOT NULL DEFAULT 0,
    owner       TEXT
);

CREATE TABLE IF NOT EXISTS parameters (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    pid         TEXT    NOT NULL UNIQUE,   -- OBD-II PID or custom label
    name        TEXT    NOT NULL,
    unit        TEXT    NOT NULL,
    min_value   REAL,
    max_value   REAL,
    description TEXT
);

CREATE TABLE IF NOT EXISTS readings (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    vehicle_id   INTEGER NOT NULL REFERENCES vehicles(id),
    parameter_id INTEGER NOT NULL REFERENCES parameters(id),
    recorded_at  TEXT    NOT NULL,         -- ISO-8601
    value        REAL    NOT NULL
);

CREATE TABLE IF NOT EXISTS maintenance (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    vehicle_id  INTEGER NOT NULL REFERENCES vehicles(id),
    service_at  TEXT    NOT NULL,          -- ISO-8601 date
    odometer_km REAL    NOT NULL,
    service     TEXT    NOT NULL,
    technician  TEXT,
    cost_usd    REAL
);

CREATE INDEX IF NOT EXISTS idx_readings_vehicle   ON readings(vehicle_id);
CREATE INDEX IF NOT EXISTS idx_readings_parameter ON readings(parameter_id);
CREATE INDEX IF NOT EXISTS idx_readings_time      ON readings(recorded_at);
CREATE INDEX IF NOT EXISTS idx_maintenance_vehicle ON maintenance(vehicle_id);
"""

VEHICLES = [
    ("1HGCM82633A123456", "Honda",      "Civic",        2019, "Silver",  "gasoline", 48200, "Alice Martin"),
    ("2T1BURHE0JC065432", "Toyota",     "Corolla",      2021, "White",   "gasoline", 31500, "Bob Singh"),
    ("5YJSA1DN9DFP12345", "Tesla",      "Model S",      2022, "Black",   "electric",  9800, "Carol Wang"),
    ("WBA3A5C55DF123456", "BMW",        "320i",         2018, "Blue",    "gasoline", 72100, "David Kim"),
    ("WAUZZZ8KXBA123456", "Audi",       "A4",           2020, "Gray",    "gasoline", 55300, "Eva Müller"),
    ("1FTFW1EF5EFA12345", "Ford",       "F-150",        2017, "Red",     "gasoline", 91000, "Frank Lopez"),
    ("1G1ZB5ST8JF123456", "Chevrolet",  "Malibu",       2019, "White",   "gasoline", 44700, "Grace Patel"),
    ("YV1RS592X92123456", "Volvo",      "S60",          2020, "Silver",  "hybrid",   28900, "Henry Zhao"),
    ("JTDKARFU8G3123456", "Toyota",     "Prius",        2016, "Green",   "hybrid",   83400, "Isabel Rossi"),
    ("1N4BL4DV8KC123456", "Nissan",     "Altima",       2021, "Black",   "gasoline", 19200, "James O'Brien"),
]

PARAMETERS = [
    ("0D",   "Vehicle Speed",              "km/h",  0,    250,  "Current vehicle speed"),
    ("0C",   "Engine RPM",                 "rpm",   0,   8000,  "Engine revolutions per minute"),
    ("05",   "Coolant Temperature",        "°C",   -40,   215,  "Engine coolant temperature"),
    ("0F",   "Intake Air Temperature",     "°C",   -40,   215,  "Air temperature at intake manifold"),
    ("10",   "Mass Air Flow",              "g/s",   0,    655,  "Mass of air entering the engine"),
    ("11",   "Throttle Position",          "%",     0,    100,  "Absolute throttle position"),
    ("2F",   "Fuel Tank Level",            "%",     0,    100,  "Remaining fuel in tank"),
    ("5C",   "Engine Oil Temperature",     "°C",   -40,   215,  "Engine oil temperature"),
    ("67",   "Coolant Temperature (alt)",  "°C",   -40,   215,  "Alternate coolant temperature sensor"),
    ("BATT", "Battery Voltage",            "V",     8,     16,  "12 V battery voltage"),
    ("TPFL", "Tyre Pressure Front-Left",   "kPa",  100,   400,  "Front-left tyre pressure"),
    ("TPFR", "Tyre Pressure Front-Right",  "kPa",  100,   400,  "Front-right tyre pressure"),
    ("TPRL", "Tyre Pressure Rear-Left",    "kPa",  100,   400,  "Rear-left tyre pressure"),
    ("TPRR", "Tyre Pressure Rear-Right",   "kPa",  100,   400,  "Rear-right tyre pressure"),
    ("LOAD", "Engine Load",                "%",     0,    100,  "Calculated engine load value"),
]

SERVICES = [
    "Oil & filter change",
    "Air filter replacement",
    "Tyre rotation",
    "Brake pad replacement",
    "Spark plug replacement",
    "Transmission fluid change",
    "Coolant flush",
    "Battery replacement",
    "Cabin air filter replacement",
    "Annual inspection",
]

TECHNICIANS = ["Mike R.", "Sarah T.", "James W.", "Priya K.", "Tom N."]

def gen_reading(pid, lo, hi):
    """Produce a plausible reading for a given PID."""
    if pid == "0D":   return round(random.uniform(0, 130), 1)
    if pid == "0C":   return round(random.uniform(700, 5500), 0)
    if pid == "05":   return round(random.uniform(80, 100), 1)
    if pid == "0F":   return round(random.uniform(15, 40), 1)
    if pid == "10":   return round(random.uniform(2, 25), 2)
    if pid == "11":   return round(random.uniform(5, 80), 1)
    if pid == "2F":   return round(random.uniform(10, 95), 1)
    if pid == "5C":   return round(random.uniform(85, 110), 1)
    if pid == "67":   return round(random.uniform(80, 100), 1)
    if pid == "BATT": return round(random.uniform(12.0, 14.8), 2)
    if pid in ("TPFL","TPFR","TPRL","TPRR"): return round(random.uniform(200, 250), 1)
    if pid == "LOAD": return round(random.uniform(10, 75), 1)
    return round(random.uniform(lo or 0, hi or 100), 2)

def main():
    if os.path.exists(DB):
        os.remove(DB)

    con = sqlite3.connect(DB)
    con.executescript(SCHEMA)

    # vehicles
    con.executemany(
        "INSERT INTO vehicles (vin,make,model,year,color,fuel_type,odometer_km,owner) "
        "VALUES (?,?,?,?,?,?,?,?)",
        VEHICLES,
    )

    # parameters
    con.executemany(
        "INSERT INTO parameters (pid,name,unit,min_value,max_value,description) "
        "VALUES (?,?,?,?,?,?)",
        PARAMETERS,
    )
    con.commit()

    # readings – 50 rows per vehicle, spread over last 30 days
    vehicle_ids   = [r[0] for r in con.execute("SELECT id FROM vehicles").fetchall()]
    param_rows    = con.execute("SELECT id, pid, min_value, max_value FROM parameters").fetchall()

    readings = []
    base = datetime.now() - timedelta(days=30)
    random.seed(42)

    for v_id in vehicle_ids:
        for _ in range(50):
            ts = base + timedelta(minutes=random.randint(0, 43200))
            p_id, pid, lo, hi = random.choice(param_rows)
            readings.append((v_id, p_id, ts.strftime("%Y-%m-%dT%H:%M:%S"), gen_reading(pid, lo, hi)))

    con.executemany(
        "INSERT INTO readings (vehicle_id, parameter_id, recorded_at, value) VALUES (?,?,?,?)",
        readings,
    )

    # maintenance – 3-8 records per vehicle
    maint = []
    base_date = datetime.now() - timedelta(days=730)
    for v_id, *_, odometer, _ in [
        (i + 1,) + row for i, row in enumerate(VEHICLES)
    ]:
        n = random.randint(3, 8)
        km = odometer - n * random.uniform(8000, 12000)
        for _ in range(n):
            km = max(0, km + random.uniform(8000, 12000))
            svc_date = base_date + timedelta(days=random.randint(0, 700))
            maint.append((
                v_id,
                svc_date.strftime("%Y-%m-%d"),
                round(km, 0),
                random.choice(SERVICES),
                random.choice(TECHNICIANS),
                round(random.uniform(40, 650), 2),
            ))

    con.executemany(
        "INSERT INTO maintenance (vehicle_id, service_at, odometer_km, service, technician, cost_usd) "
        "VALUES (?,?,?,?,?,?)",
        maint,
    )
    con.commit()

    # summary
    for table in ("vehicles", "parameters", "readings", "maintenance"):
        count = con.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
        print(f"  {table:<15} {count:>4} rows")

    con.close()
    print(f"\nDatabase written to: {DB}")

if __name__ == "__main__":
    main()
