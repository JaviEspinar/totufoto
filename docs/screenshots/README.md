# Screenshots

The README's screenshots, taken by `web/tests/screenshots.mjs` (Playwright) of a gallery on the library described here:

```sh
cd web/tests && npm ci && npx playwright install chromium
node screenshots.mjs http://127.0.0.1:7878
```

## The library

119 photos and 3 short videos of NASA's Artemis II mission, from the crew's training in 2023 to their flight around the Moon in April 2026 and the tour after it, taken from the [NASA Image and Video Library](https://images.nasa.gov). NASA's photos and videos are **not protected by copyright** in the United States (works of the US government). NASA asks to be credited, and that its pictures aren't used to suggest NASA endorses anything: it doesn't endorse Imadive. The crew are Reid Wiseman, Victor Glover and Christina Koch of NASA, and Jeremy Hansen of the Canadian Space Agency.

The library lived in `/Users/Shared/Photos`, one folder per month and place. To show the gallery as it is used:

- **Photos** are 1600 pixels on their long side, named after their NASA title, and given EXIF data:
  - **Date**: the day of NASA's record, from 11:00 in the record's order (the records give no time).
  - **Place**: the coordinates of where the record says the photo was taken (Kennedy Space Center, Houston, Washington...). The gallery shows the nearest place with more than 1,000 inhabitants, so Kennedy Space Center appears as Merritt Island. Photos taken at sea or in Orion have no place, as a phone's would.
- **Videos** are 12-second clips (H.264 and AAC, 1280 pixels wide), with their date and place in the QuickTime keys a phone writes. Their thumbnails were made by the page in Chrome, which plays H.264; the test browser doesn't.
- **People**: the faces were found and grouped by the gallery itself. The four largest groups were named after the crew, checked against the photos whose NASA title names one astronaut. Six small groups of other people (2 photos each) were removed from the index, so that only the crew is shown.

## Videos

| Folder | Video (NASA) | Clip |
|---|---|---|
| 2026/03 Kennedy Space Center | [Rollout from the VAB](https://images.nasa.gov/details/KSC-20260320-MH-GEB01-0001-Artemis_II_Rollout_For_Launch-M19827) | 12 seconds from 56 s |
| 2026/04 Kennedy Space Center | [Launch slow motion](https://images.nasa.gov/details/KSC-04012026-C4829) | 12 seconds from 26 s |
| 2026/04 At sea | [Splashdown](https://images.nasa.gov/details/KSC-20260410-MH-JBP01-0001-Artemis_II_Splashdown_GSS_Visual_60fps-M19183) | 12 seconds from 140 s |

## Photos

| Folder | Photo (NASA) | Photographer |
|---|---|---|
| 2023/03 Houston | [Artemis II crew portrait](https://images.nasa.gov/details/jsc2023e016432_alt2) | Josh Valcarcel – NASA JSC |
| 2023/04 Houston | [Artemis II crew portrait](https://images.nasa.gov/details/jsc2023e016431_alt) | ROBERT MARKOWITZ  NASA-JSC |
| 2023/05 Greenbelt | [Artemis II Crew at GSFC 1](https://images.nasa.gov/details/NHQ202305190040) | NASA/Joel Kowsky |
| 2023/05 Greenbelt | [Artemis II Crew at GSFC 2](https://images.nasa.gov/details/NHQ202305190048) | NASA/Joel Kowsky |
| 2023/05 Washington | [Artemis II Crew Media Availability 1](https://images.nasa.gov/details/NHQ202305180013) | NASA/Bill Ingalls |
| 2023/05 Washington | [Artemis II Crew Media Availability 2](https://images.nasa.gov/details/NHQ202305180023) | NASA/Bill Ingalls |
| 2023/05 Washington | [Artemis II Crew Media Availability 3](https://images.nasa.gov/details/NHQ202305180030) | NASA/Bill Ingalls |
| 2023/05 Washington | [Artemis II Crew Senate Meet and Greet 1](https://images.nasa.gov/details/NHQ202305170009) | NASA/Bill Ingalls |
| 2023/05 Washington | [Artemis II Crew Senate Meet and Greet 2](https://images.nasa.gov/details/NHQ202305170005) | NASA/Bill Ingalls |
| 2023/05 Washington | [Artemis II Crew Senate Meet and Greet 3](https://images.nasa.gov/details/NHQ202305170003) | NASA/Bill Ingalls |
| 2023/07 San Diego | [Artemis II Crew Visits Naval Base San Diego](https://images.nasa.gov/details/KSC-20230719-PH-JNS01_0001) | U.S. Navy Photo by Mass Communic |
| 2023/08 Kennedy Space Center | [KSC Orion Media Day - Artemis II Crew 1](https://images.nasa.gov/details/KSC-20230808-PH-KLS02_0093) | NASA/Kim Shiflett |
| 2023/08 Kennedy Space Center | [KSC Orion Media Day - Artemis II Crew 2](https://images.nasa.gov/details/KSC-20230808-PH-KLS02_0086) | NASA/Kim Shiflett |
| 2023/08 Kennedy Space Center | [KSC Orion Media Day - Artemis II Crew 3](https://images.nasa.gov/details/KSC-20230808-PH-KLS02_0098) | NASA/Kim Shiflett |
| 2023/08 Kennedy Space Center | [NASA Briefing with Artemis II Crew 1](https://images.nasa.gov/details/KSC-20230808-PH-KLS04_0115) | NASA/Kim Shiflett |
| 2023/08 Kennedy Space Center | [NASA Briefing with Artemis II Crew 2](https://images.nasa.gov/details/KSC-20230808-PH-KLS04_0027) | NASA/Kim Shiflett |
| 2023/08 Kennedy Space Center | [NASA Briefing with Artemis II Crew 3](https://images.nasa.gov/details/KSC-20230808-PH-KLS04_0014) | NASA/Kim Shiflett |
| 2023/11 Huntsville | [ARTEMIS II CREW VISIT](https://images.nasa.gov/details/MSFC-202301280) | Charles Beason |
| 2024/01 Houston | [Artemis II crew portrait 1](https://images.nasa.gov/details/jsc2024e010107) | Josh Valcarcel – NASA – John |
| 2024/01 Houston | [Artemis II crew portrait 2](https://images.nasa.gov/details/jsc2024e009647) | Josh Valcarcel – NASA – John |
| 2024/01 Houston | [Artemis II crew portrait 3](https://images.nasa.gov/details/jsc2024e009650) | Josh Valcarcel – NASA – John |
| 2024/01 Houston | [Artemis II crew portrait 4](https://images.nasa.gov/details/jsc2024e010118) | Josh Valcarcel – NASA – John |
| 2024/02 At sea | [Artemis II Orion Underway Recovery Test 11 URT-11 - Day 5 1](https://images.nasa.gov/details/KSC-20240225-PH-ILW01-0128) | NASA/Isaac Watson |
| 2024/02 At sea | [Artemis II Orion Underway Recovery Test 11 URT-11 - Day 5 2](https://images.nasa.gov/details/KSC-20240225-PH-ILW01-0186) | NASA/Isaac Watson |
| 2024/06 Washington | [Artemis II Astronauts Participate in Moon Tree Dedication Ceremo 1](https://images.nasa.gov/details/NHQ202406040011) | NASA/Aubrey Gemignani |
| 2024/06 Washington | [Artemis II Astronauts Participate in Moon Tree Dedication Ceremo 2](https://images.nasa.gov/details/NHQ202406040012) | NASA/Aubrey Gemignani |
| 2024/06 Washington | [Artemis II Astronauts Participate in Moon Tree Dedication Ceremo 3](https://images.nasa.gov/details/NHQ202406040022) | NASA/Aubrey Gemignani |
| 2024/06 Washington | [Black Space Week 2024 at the NMAAHC 1](https://images.nasa.gov/details/NHQ202406170015) | NASA/Aubrey Gemignani |
| 2024/06 Washington | [Black Space Week 2024 at the NMAAHC 2](https://images.nasa.gov/details/NHQ202406170017) | NASA/Aubrey Gemignani |
| 2024/07 Iceland | [Artemis II Crew geology training in Iceland 1](https://images.nasa.gov/details/jsc2024e054663) | ROBERT MARKOWITZ  NASA-JSC |
| 2024/07 Iceland | [Artemis II Crew geology training in Iceland 2](https://images.nasa.gov/details/jsc2024e055031) | ROBERT MARKOWITZ  NASA-JSC |
| 2024/07 New Orleans | [NASA Astronaut Victor Glover Views Artemis II Rocket Stage at NASA Mic](https://images.nasa.gov/details/MAF_20240715_CS2_VictorGlover_MD_15) | NASA/Michael DeMocker |
| 2024/09 Sandusky | [Orion Technical Visit and Artemis II All Hands](https://images.nasa.gov/details/GRC-2024-C-09996) | NASA/Sara Lowthian-Hanna |
| 2024/11 Kennedy Space Center | [Artemis II Crew Visits KSC 1](https://images.nasa.gov/details/KSC-20241119-PH-JBS02_0162) | NASA/Ben Smegelsky |
| 2024/11 Kennedy Space Center | [Artemis II Crew Visits KSC 2](https://images.nasa.gov/details/KSC-20241119-PH-JBS01_0063) | NASA/Ben Smegelsky |
| 2024/11 Rockledge | [Artemis II Crew Imagery - Victor Glover at JP Donovan 1](https://images.nasa.gov/details/KSC-20241108-PH-JBS01_0065) | NASA/Ben Smegelsky |
| 2024/11 Rockledge | [Artemis II Crew Imagery - Victor Glover at JP Donovan 2](https://images.nasa.gov/details/KSC-20241108-PH-JBS01_0075) | NASA/Ben Smegelsky |
| 2024/12 Washington | [NASA Agencywide All Hands](https://images.nasa.gov/details/NHQ202412060028) | NASA/Bill Ingalls |
| 2024/12 Washington | [NASA Artemis II Briefing](https://images.nasa.gov/details/NHQ202412050010) | NASA/Bill Ingalls |
| 2025/01 Houston | [NASA astronaut and Artemis II mission specialist Christina Koch exits](https://images.nasa.gov/details/jsc2025e004089) | Mark Sowa - NASA - JSC |
| 2025/01 Houston | [The Artemis II crew completing Post Insertion and Deorbit Preparation](https://images.nasa.gov/details/jsc2025e004086) | Mark Sowa - NASA - JSC |
| 2025/03 At sea | [NASA Artemis Underway Recovery Test 12 1](https://images.nasa.gov/details/NHQ202503280064) | NASA/Joel Kowsky |
| 2025/03 At sea | [NASA Artemis Underway Recovery Test 12 2](https://images.nasa.gov/details/NHQ202503280066) | NASA/Joel Kowsky |
| 2025/03 At sea | [NASA Artemis Underway Recovery Test 12 3](https://images.nasa.gov/details/NHQ202503290025) | NASA/Joel Kowsky |
| 2025/07 Kennedy Space Center | [Artemis II Suit Crew Test and CEIT 1](https://images.nasa.gov/details/KSC-20250731-PH-RNS01_0007) | NASA/Rad Sinyak |
| 2025/07 Kennedy Space Center | [Artemis II Suit Crew Test and CEIT 2](https://images.nasa.gov/details/KSC-20250731-PH-RNS01_0003) | NASA/Rad Sinyak |
| 2025/07 Kennedy Space Center | [Artemis II Suit Crew Test and CEIT 3](https://images.nasa.gov/details/KSC-20250731-PH-RNS01_0004) | NASA/Rad Sinyak |
| 2025/08 Kennedy Space Center | [Artemis II Crew Suiting and Walkout 1](https://images.nasa.gov/details/KSC-20250811-PH-KLS01_0074) | NASA/Kim Shiflett |
| 2025/08 Kennedy Space Center | [Artemis II Crew Suiting and Walkout 2](https://images.nasa.gov/details/KSC-20250811-PH-KLS01_0049) | NASA/Kim Shiflett |
| 2025/08 Kennedy Space Center | [Artemis II Night Runs Dry Dress Rehearsals](https://images.nasa.gov/details/KSC-20250812-PH-KLS01_0106) | NASA/Kim Shiflett |
| 2025/10 Kennedy Space Center | [Artemis II Orion Prep for Integration to SLS 1](https://images.nasa.gov/details/KSC-20251017-PH-AJN01_0063) | NASA/Amber Jean Notvest |
| 2025/10 Kennedy Space Center | [Artemis II Orion Prep for Integration to SLS 2](https://images.nasa.gov/details/KSC-20251017-PH-AJN01_0060) | NASA/Amber Jean Notvest |
| 2025/12 Kennedy Space Center | [Artemis Triage Site End to End Runs 1](https://images.nasa.gov/details/NHQ202512190020) | NASA/Joel Kowsky |
| 2025/12 Kennedy Space Center | [Artemis Triage Site End to End Runs 2](https://images.nasa.gov/details/NHQ202512190033) | NASA/Joel Kowsky |
| 2025/12 Kennedy Space Center | [Artemis Triage Site End to End Runs 3](https://images.nasa.gov/details/NHQ202512190055) | NASA/Joel Kowsky |
| 2026/01 Kennedy Space Center | [Artemis II Rollout 1](https://images.nasa.gov/details/NHQ202601170106) | NASA/Joel Kowsky |
| 2026/01 Kennedy Space Center | [Artemis II Rollout 2](https://images.nasa.gov/details/KSC-20260117-PH-KLS03_0204) | NASA/Kim Shiflett |
| 2026/01 Kennedy Space Center | [Artemis II Rollout 3](https://images.nasa.gov/details/KSC-20260117-PH-KLS03_0016) | NASA/Kim Shiflett |
| 2026/01 Kennedy Space Center | [Artemis II Rollout 4](https://images.nasa.gov/details/KSC-20260117-PH-KLS03_0026) | NASA/Kim Shiflett |
| 2026/01 Kennedy Space Center | [Artemis II Sunrise](https://images.nasa.gov/details/KSC-20260128-PH-CSH01-0050) | NASA/Cory S Huston |
| 2026/01 Kennedy Space Center | [Artemis II at Launch Pad 39B](https://images.nasa.gov/details/KSC-01282026-Artemis%20II_Moon%20Shots-12) | NASA/Brandon Hancock |
| 2026/01 Kennedy Space Center | [Artemis II rollout 5](https://images.nasa.gov/details/NHQ20260117_admin_0006) | NASA/John Kraus |
| 2026/01 Kennedy Space Center | [Artemis II](https://images.nasa.gov/details/KSC-20260117-PH-JBS01_0024) | NASA/Ben Smegelsky |
| 2026/02 Kennedy Space Center | [Artemis II Preflight 1](https://images.nasa.gov/details/NHQ20260201_admin_0008) | NASA/John Kraus |
| 2026/02 Kennedy Space Center | [Artemis II Preflight 2](https://images.nasa.gov/details/NHQ20260201_admin_0013) | NASA/John Kraus |
| 2026/02 Kennedy Space Center | [Artemis II on Launch Pad 1](https://images.nasa.gov/details/KSC-20260210-PH-JBS01-0115) | NASA/Ben Smegelsky |
| 2026/02 Kennedy Space Center | [Artemis II on Launch Pad 2](https://images.nasa.gov/details/KSC-20260210-PH-JBS01-0126) | NASA/Ben Smegelsky |
| 2026/03 Kennedy Space Center | [Artemis II Crew Arrive At Kennedy Space Center 1](https://images.nasa.gov/details/AFRC2026-0064-13) | NASA/Jim Ross |
| 2026/03 Kennedy Space Center | [Artemis II Crew Arrive At Kennedy Space Center 2](https://images.nasa.gov/details/AFRC2026-0064-06) | NASA/Jim Ross |
| 2026/03 Kennedy Space Center | [Artemis II Crew Arrives at Kennedy](https://images.nasa.gov/details/NHQ20260327_admin_0003) | NASA/John Kraus |
| 2026/03 Kennedy Space Center | [Artemis II Crew Zap The Wall and Patch at Walkout 1](https://images.nasa.gov/details/KSC-20260330-PH-KLS01_0017) | NASA/Kim Shiflett |
| 2026/03 Kennedy Space Center | [Artemis II Crew Zap The Wall and Patch at Walkout 2](https://images.nasa.gov/details/KSC-20260330-PH-KLS01_0055) | NASA/Kim Shiflett |
| 2026/03 Kennedy Space Center | [Artemis II Launch Crew Arrival at KSC 1](https://images.nasa.gov/details/KSC-20260327-PH-KLS01_0284) | NASA/Kim Shiflett |
| 2026/03 Kennedy Space Center | [Artemis II Launch Crew Arrival at KSC 2](https://images.nasa.gov/details/KSC-20260327-PH-KLS01_0149) | NASA/Kim Shiflett |
| 2026/03 Kennedy Space Center | [Artemis II Preflight 1](https://images.nasa.gov/details/NHQ202603300017) | NASA/Bill Ingalls |
| 2026/03 Kennedy Space Center | [Artemis II Preflight 2](https://images.nasa.gov/details/NHQ202603300010) | NASA/Bill Ingalls |
| 2026/03 Kennedy Space Center | [Artemis II Rollout for Launch](https://images.nasa.gov/details/KSC-20260320-PH-JBS01_0118) | NASA/Ben Smegelsky |
| 2026/03 Titusville | [Artemis II Sunrise at Max Brewer Bridge](https://images.nasa.gov/details/KSC-20260324-PH-JBS01_0016) | NASA/Ben Smegelsky |
| 2026/04 At sea | [Artemis II Crew Recovery 1](https://images.nasa.gov/details/JB5_0814) | NASA \ James Blair |
| 2026/04 At sea | [Artemis II Crew Recovery 2](https://images.nasa.gov/details/JB5_0855) | NASA / James Blair |
| 2026/04 Coronado | [Artemis II Recovery 1](https://images.nasa.gov/details/NHQ202604110106) | NASA/Keegan Barber |
| 2026/04 Coronado | [Artemis II Recovery 2](https://images.nasa.gov/details/NHQ202604110109) | NASA/Keegan Barber |
| 2026/04 Coronado | [Artemis II Recovery 3](https://images.nasa.gov/details/NHQ202604110107) | NASA/Keegan Barber |
| 2026/04 Coronado | [Artemis II Recovery 4](https://images.nasa.gov/details/NHQ202604110108) | NASA/Keegan Barber |
| 2026/04 Houston | [Artemis II Crew Return](https://images.nasa.gov/details/jsc2026e022292) | NASA/Bill Stafford |
| 2026/04 Kennedy Space Center | [Artemis II Launch 3](https://images.nasa.gov/details/KSC-04012026-Artemis%20II_Launch-4) | NASA/Brandon Hancock |
| 2026/04 Kennedy Space Center | [Artemis II Launch Crew Walkout 1](https://images.nasa.gov/details/NHQ20260401_admin_0007) | NASA/John Kraus |
| 2026/04 Kennedy Space Center | [Artemis II Launch Crew Walkout 2](https://images.nasa.gov/details/NHQ20260401_admin_0006) | NASA/John Kraus |
| 2026/04 Kennedy Space Center | [Artemis II Launch Crew Walkout 3](https://images.nasa.gov/details/NHQ20260401_admin_0001) | NASA/John Kraus |
| 2026/04 Kennedy Space Center | [Artemis II Launch Crew Walkout 4](https://images.nasa.gov/details/NHQ20260401_admin_0003) | NASA/John Kraus |
| 2026/04 Kennedy Space Center | [Artemis II Launch Day, Crew Walkout](https://images.nasa.gov/details/KSC-20260401-PH-KLS02_0139) | NASA/Kim Shiflett |
| 2026/04 Kennedy Space Center | [Artemis II Walkout 1](https://images.nasa.gov/details/NHQ202604010020) | NASA/Aubrey Gemignani |
| 2026/04 Kennedy Space Center | [Artemis II Walkout 2](https://images.nasa.gov/details/NHQ202604010021) | NASA/Aubrey Gemignani |
| 2026/04 Kennedy Space Center | [Artemis II launch 1](https://images.nasa.gov/details/SLS_MAF_20260401_ArtemisIILaunch_05) | NASA/Michael DeMocker |
| 2026/04 Kennedy Space Center | [Artemis II launch 2](https://images.nasa.gov/details/SLS_MAF_20260401_ArtemisIILaunch_07) | NASA/Michael DeMocker |
| 2026/04 New York | [NASA Artemis II Crew Rings Nasdaq Closing Bell 1](https://images.nasa.gov/details/NHQ202604300030) | NASA/Bill Ingalls |
| 2026/04 New York | [NASA Artemis II Crew Rings Nasdaq Closing Bell 2](https://images.nasa.gov/details/NHQ202604300022) | NASA/Bill Ingalls |
| 2026/04 Orion | [Christina Koch Readies for Saliva Sample Collection](https://images.nasa.gov/details/art002e027803) | NASA |
| 2026/04 Orion | [Final Flyby Preparations](https://images.nasa.gov/details/art002e009294) | NASA |
| 2026/04 Orion | [Home, Seen from Orion](https://images.nasa.gov/details/art002e009007) | NASA |
| 2026/04 Orion | [Lunar Flyby Observations  CSA astronaut Jeremy Hansen](https://images.nasa.gov/details/art002e016171) | NASA |
| 2026/04 Orion | [Lunar Selfie](https://images.nasa.gov/details/art002e009296) | NASA |
| 2026/04 Orion | [Moon Joy](https://images.nasa.gov/details/art002e013367) | NASA |
| 2026/04 Orion | [Reid Wiseman Poses With Earth in View](https://images.nasa.gov/details/art002e041724) | NASA |
| 2026/04 Orion | [Victor Glover Poses With Earth in View](https://images.nasa.gov/details/art002e041740) | NASA |
| 2026/05 Washington | [Artemis II Crew on Capitol Hill 1](https://images.nasa.gov/details/NHQ202605120026) | NASA/Joel Kowsky |
| 2026/05 Washington | [Artemis II Crew on Capitol Hill 2](https://images.nasa.gov/details/NHQ202605120024) | NASA/Joel Kowsky |
| 2026/05 Washington | [Artemis II Crew on Capitol Hill 3](https://images.nasa.gov/details/NHQ202605120073) | NASA/Aubrey Gemignani |
| 2026/05 Washington | [Artemis II Crew on Capitol Hill 4](https://images.nasa.gov/details/NHQ202605120074) | NASA/Aubrey Gemignani |
| 2026/06 Houston | [Artemis II Employee Celebration Event 1](https://images.nasa.gov/details/jsc2026e397494) | Morgan Gridley NASA-JSC |
| 2026/06 Houston | [Artemis II Employee Celebration Event 2](https://images.nasa.gov/details/jsc2026e396705) | James Blair - NASA - JSC |
| 2026/06 Houston | [Artemis II Employee Celebration Event 3](https://images.nasa.gov/details/jsc2026e396751) | James Blair - NASA - JSC |
| 2026/06 Houston | [Artemis II crew portrait](https://images.nasa.gov/details/jsc2026e392665) | Robert Markowitz |
| 2026/07 Washington | [Artemis II Crew at Nationals Park 1](https://images.nasa.gov/details/NHQ202607040004) | NASA/Bill Ingalls |
| 2026/07 Washington | [Artemis II Crew at Nationals Park 2](https://images.nasa.gov/details/NHQ202607040008) | NASA/Bill Ingalls |
| 2026/09 Huntsville | [Artemis II Crew Visits Marshall 1](https://images.nasa.gov/details/130A4387-1) | NASA/Savannah Bullard |
| 2026/09 Huntsville | [Artemis II Crew Visits Marshall 2](https://images.nasa.gov/details/130A4323-1) | NASA/Savannah Bullard |
| 2026/09 Huntsville | [Artemis II Crew Visits Marshall 3](https://images.nasa.gov/details/130A4831-1) | NASA/Savannah Bullard |
| 2026/10 Philadelphia | [NASAs Inspiration Tour at Lincoln Financial Field](https://images.nasa.gov/details/NHQ202610040009) | NASA/Thalia Patrinos |
