#!/usr/bin/env python3
"""Build ASL S1-S13 season folders as copies of the matching map files."""
import os, shutil, sys

M = "/mnt/games/Games/cosmonarchy/prefix/drive_c/Cosmonarchy/Starcraft/Maps/CMBW-Root/CMBW-maps/melee/1v1"
FLAT = os.path.join(M, "ASL")
T50 = os.path.join(M, "Top 50")
ASL = os.path.join(M, "ASL")

# pool map name -> source file (None = not available)
POOLS = {
 "S1":  {"Circuit Breaker": ("t50", "4CircuitBreakers1.0.scx"),
         "Fighting Spirit": ("fs", None),
         "Taebaek Mountains": ("flat", "(2)NEWTAEBAEK.scx"),
         "Overwatch": ("flat", "(4)OVERWATCH1.3.scx")},
 "S2":  {"Benzene": ("t50", "(2)Benzene1.1.SCX"),
         "Circuit Breaker": ("t50", "4CircuitBreakers1.0.scx"),
         "Destination": ("t50", "(2)Destination1.1.scx"),
         "Eye of the Storm": ("missing", None),
         "Demian": ("flat", "(3)Demian_SE_2.0.scx")},
 "S3":  {"Circuit Breaker": ("t50", "4CircuitBreakers1.0.scx"),
         "Outsider": ("t50", "3Outsider_SE_2.2.scx"),
         "Blue Storm": ("t50", "(2)BlueStorm1.2.scx"),
         "Camelot": ("flat", "(2)Camelot_1.6.scx"),
         "Andromeda": ("t50", "(4)Andromeda_1.2.scx")},
 "S4":  {"Tau Cross": ("t50", "(3)Tau_Cross1.1.scx"),
         "Fighting Spirit": ("fs", None),
         "Gladiator": ("t50", "(4)Gladiator1.1.scx"),
         "Crossing Field": ("flat", "(2)Crossing_Field_1.34.scx"),
         "Gold Rush": ("flat", "(3)GoldRush1.1.scx")},
 "S5":  {"Gladiator": ("t50", "(4)Gladiator1.1.scx"),
         "Sparkle": ("flat", "(4)Sparkle_1.1.scx"),
         "Transistor": ("missing", None),
         "Third World": ("flat", "(2)ThirdWorld1.0.scx")},
 "S6":  {"Autobahn": ("flat", "(2)Autobahn_1.1.scx"),
         "Circuit Breaker": ("t50", "4CircuitBreakers1.0.scx"),
         "Sylphid": ("missing", None),
         "Neo Transistor": ("flat", "(3)Neo_Transistor2.0.scm")},
 "S7":  {"Block Chain": ("missing", None),
         "Whiteout": ("flat", "(3)Whiteout1.2.scx"),
         "Neo Sylphid": ("flat", "(3)Neo_Sylphid_2.0.scx"),
         "Match Point": ("t50", "(2)MatchPoint1.3.scx")},
 "S8":  {"Block Chain SE": ("flat", "(4)BlockChainSE2.1.scx"),
         "Multiverse": ("flat", "(4)Multiverse1.0.scx"),
         "Neo Ground Zero": ("t50", "(4)Neo_Ground_Zero2.0.scx"),
         "Neo Sylphid": ("flat", "(3)Neo_Sylphid_2.0.scx"),
         "New Bloody Ridge": ("t50", "(2)New_Bloody_Ridge2.1.scx"),
         "Overwatch": ("flat", "(4)OVERWATCH1.3.scx"),
         "Tripod": ("flat", "(3)Tripod1.2.scm")},
 "S9":  {"Neo Sylphid": ("flat", "(3)Neo_Sylphid_2.0.scx"),
         "Escalade": ("flat", "(4)Escalade1.0.scx"),
         "Inner Coven": ("flat", "(3)Inner_Coven_1.2.scx"),
         "Horizon Lunar Colony": ("flat", "(4)Horizon_lunar_colony1.1.scm"),
         "Polypoid": ("flat", "(4)Polypoid_1.65.scx"),
         "Matchpoint": ("t50", "(2)MatchPoint1.3.scx"),
         "Hitchhiker": ("t50", "(2)Hitchhiker+1.4.scx")},
 "S10": {"Benzene": ("t50", "(2)Benzene1.1.SCX"),
         "Eclipse": ("flat", "(2)Eclipse_1.2.scx"),
         "Optimizer": ("flat", "(4)Optimizer1.0.scx"),
         "Plasma": ("t50", "(3)Plasma_1.0.scx"),
         "Polypoid": ("flat", "(4)Polypoid_1.65.scx"),
         "Ringing Bloom": ("flat", "(4)Ringing_Bloom_SE_2.0.scx"),
         "Shakuras Temple": ("flat", "(4)Shakuras_Temple1.0.scx")},
 "S11": {"Polaris Rhapsody": ("t50", "(2)Polaris_Rhapsody_1.0.scx"),
         "Fighting Spirit": ("fs", None),
         "Eclipse": ("flat", "(2)Eclipse_1.2.scx"),
         "Polypoid": ("flat", "(4)Polypoid_1.65.scx"),
         "Ascension": ("flat", "(3)Ascension_1.0.scx"),
         "Hidden Track": ("flat", "(4)HiddenTrack1.2.scx"),
         "Ultimate Stream": ("flat", "(4)Ultimate_Stream_1.0.scx")},
 "S12": {"Ascension": ("flat", "(3)Ascension_1.0.scx"),
         "Eclipse": ("flat", "(2)Eclipse_1.2.scx"),
         "GOOD NIGHT": ("flat", "(4)GOOD_NIGHT_1.31.scx"),
         "Largo": ("flat", "(4)Largo_1.5.scx"),
         "Lemon": ("flat", "(4)Lemon_1.1.scx"),
         "Polypoid": ("flat", "(4)Polypoid_1.65.scx"),
         "Revolver": ("flat", "(4)Revolver1.0.scx")},
 "S13": {"Revolver SE": ("missing", None),
         "Vermeer": ("missing", None),
         "Allegro": ("s14", "(4)Allegro_11c.scx"),
         "Eclipse": ("flat", "(2)Eclipse_1.2.scx"),
         "Butter": ("s14", "(2)Butter_20c.scx"),
         "Metaverse": ("missing", None),
         "Monopoly": ("missing", None)},
}

def resolve(kind, fname):
    if kind == "t50":
        return os.path.join(T50, fname)
    if kind == "flat":
        return os.path.join(FLAT, fname)
    if kind == "s14":
        return os.path.join(ASL, "S14", fname)
    if kind == "fs":
        return "/tmp/opencode/mapwork/fs14.scx" if os.path.exists("/tmp/opencode/mapwork/fs14.scx") else None
    return None

def main():
    report = []
    for season in sorted(POOLS):
        d = os.path.join(ASL, season)
        os.makedirs(d, exist_ok=True)
        for mapname, (kind, fname) in POOLS[season].items():
            src = resolve(kind, fname)
            if src and os.path.exists(src):
                dst = os.path.join(d, os.path.basename(src))
                if not os.path.exists(dst):
                    shutil.copyfile(src, dst)
                report.append(f"{season}: {mapname} -> {os.path.basename(src)}")
            else:
                report.append(f"{season}: {mapname} -> MISSING")
    print("\n".join(report))
    with open("/tmp/opencode/mapwork/s1-13-report.txt", "w") as f:
        f.write("\n".join(report) + "\n")

if __name__ == "__main__":
    main()
