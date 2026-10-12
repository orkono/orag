# AquaBoost P-450 Booster Pump — User Manual

Document BWP-UM-450, revision 3, issued March 2026.

Manufacturer: Brightwater Pump Technologies Ltd., Unit 7, Harrow Mill Estate, Leeds, United Kingdom. Technical support: support@brightwaterpumps.example.

## 1. About this manual

This manual covers the AquaBoost P-450 automatic domestic booster pump with integrated pressure tank and electronic controller. Read the whole manual before installing the pump and keep it for future reference. The P-450 is designed to raise and hold the water pressure of a single household or a small drip irrigation system fed from a mains supply, a cistern or a shallow well.

The pump is identified by the serial number plate on the underside of the motor housing. The serial number has the format P450-YYWW-NNNNN, where YYWW is the production year and week.

## 2. Safety

The P-450 must be connected to a 230 V supply protected by a 30 mA residual current device (RCD). Never operate the pump without the RCD in place.

The pump is intended for clean water only. Do not pump flammable, corrosive or abrasive liquids, seawater, or water containing fibres. Water with more than 50 g/m³ of suspended solids shortens the life of the shaft seal.

Disconnect the mains plug before any maintenance work. The pressure tank remains pressurised after the pump is switched off; open a tap to release the pressure before loosening any pipe connection.

The pump must not be used by children or by persons with reduced physical or mental capability unless they are supervised. Keep the pump out of reach of children.

The maximum permitted water temperature is 40 °C. Pumping warmer water destroys the pressure sensor membrane and voids the warranty.

Never run the pump dry for longer than the built-in protection allows. The controller stops the motor after twenty seconds without flow, but repeated dry starts overheat the seal.

The pump is not frost-proof. If the installation site can fall below 2 °C, drain the pump body through the drain plug and store the pump indoors.

Do not lift the pump by the power cable or by the pressure sensor housing. Use the carrying handle moulded into the tank bracket.

## 3. Scope of delivery

The shipping carton contains the following items:

- 1 × AquaBoost P-450 pump unit with 2.0 litre pressure tank
- 1 × non-return valve G1, pre-fitted to the inlet port
- 1 × inlet strainer, mesh 0.5 mm
- 2 × G1 union couplings with flat gaskets
- 1 × wall mounting plate with four M8 anchor bolts
- 1 × this user manual

Check the carton for transport damage before installation. Damaged goods must be reported to the dealer within 48 hours of delivery.

## 4. Installation

### 4.1 Location

Install the pump in a dry, frost-free and ventilated room. Leave at least 200 mm of free space around the motor fan cover for cooling air. The ambient temperature at the installation site must stay between 4 °C and 40 °C.

The pump is rated IP44 and may be splashed but not submerged. Do not install it in a pit that can flood.

Place the pump on a level surface and fix it with the supplied wall plate or with the four rubber feet. The rubber feet reduce noise transmission to the floor and must not be removed when the pump stands on a concrete base.

### 4.2 Suction side

The maximum suction lift of the P-450 is 8 metres measured from the water surface to the pump inlet. Longer or higher suction lines cause cavitation and code E-21 trips.

Use a suction pipe of at least 25 mm internal diameter. The suction pipe must rise continuously towards the pump without any high point in which air can collect.

Fit the supplied inlet strainer at the free end of the suction pipe and keep it at least 300 mm below the lowest water level and at least 300 mm above the bottom of the tank or well.

Where the pump draws from a mains supply, the inlet pressure must not exceed 2.0 bar. Install a pressure reducing valve upstream if the mains pressure is higher.

### 4.3 Discharge side

Connect the discharge pipe to the G1 outlet port on top of the pump using one of the supplied union couplings. Support the discharge pipe so that its weight does not rest on the pump body.

Install an isolating ball valve in the discharge line within 1 metre of the pump so that the pump can be serviced without draining the whole installation.

### 4.4 Priming

Before the first start, remove the priming plug on the top of the pump body and fill the pump completely with clean water. Refit the plug hand-tight. A pump that is started unprimed will trip the dry-run protection and may damage the seal.

### 4.5 Electrical connection

The P-450 is supplied with a 1.5 metre cable and a moulded Schuko plug. The socket must be earthed and fused at 10 A. Do not use extension leads longer than 10 metres or rated below 1.5 mm².

## 5. Operation

### 5.1 Control panel

The control panel on the tank bracket has a two-digit display, a green POWER lamp, a red FAULT lamp and two buttons marked RESET and MODE.

The display shows the current system pressure in bar during normal operation. When a fault is present the display alternates between the pressure and the error code.

### 5.2 Automatic mode

In automatic mode the pump starts when the system pressure falls to the cut-in pressure of 1.5 bar and stops when flow ceases and the pressure has reached the cut-out pressure of 3.0 bar. The small pressure tank buffers short draws such as a toilet cistern filling.

The controller allows at most thirty starts per hour. If this limit is exceeded the controller holds the pump off for ten minutes and then resumes automatic operation.

### 5.3 Leak monitoring

The controller monitors the system pressure while the pump is idle. If the pump restarts more than eight times in one hour with no flow registered by the flow sensor, the controller logs code E-27 and continues to operate; the FAULT lamp flashes slowly to prompt the user to look for a dripping tap or a leaking cistern valve.

### 5.4 Manual mode

Press and hold MODE for three seconds to enter manual mode. In manual mode the pump runs continuously regardless of pressure and the display shows the letter M. Manual mode ends automatically after 15 minutes or when MODE is pressed again. Manual mode is intended for filling empty pipework and flushing new installations.

### 5.5 Restarting after a fault

A latched fault is cleared by pressing RESET once. If the same fault recurs three times within one hour the controller refuses further resets for one hour and shows the code continuously.

## 6. Error codes

The controller reports the following codes. Codes marked latched stop the pump until RESET is pressed; non-latched codes clear automatically when the condition disappears.

| Code | Meaning | Action |
|------|---------|--------|
| E-21 | Dry run: no flow and no pressure rise within 20 s of starting (latched) | Check the water supply and the suction line, prime the pump, press RESET |
| E-22 | Motor overcurrent above 3.2 A (latched) | Check for a blocked impeller or a seized bearing; call service if the code returns |
| E-23 | Supply undervoltage below 198 V | Check the mains supply and the extension lead; the pump restarts when voltage recovers |
| E-24 | Supply overvoltage above 253 V | Disconnect the pump and have the installation checked by an electrician |
| E-25 | Motor winding temperature above 85 °C (latched) | Allow the pump to cool for 30 minutes, check ventilation, press RESET |
| E-26 | Pressure sensor signal out of range | Check the sensor cable; replace the sensor if the code persists |
| E-27 | Leak suspected: repeated pressure loss while idle (non-latched) | Check all taps, cistern valves and pipe joints for drips |
| E-28 | Flow sensor paddle stuck or disconnected | Clean the flow sensor paddle; check the sensor connector |
| E-29 | Continuous run time above 120 minutes | Check for a burst pipe or an open outlet; press RESET |
| E-30 | Water temperature below 2 °C, frost risk | Stop the pump, drain the body, warm the room before restarting |
| E-31 | Communication error between control board and motor driver (latched) | Switch off at the mains for one minute and switch on again; call service if repeated |
| E-32 | Cooling fan not rotating | Remove debris from the fan cover; replace the fan if it does not turn freely |
| E-33 | Tank precharge pressure below 0.8 bar | Recharge the tank to 1.3 bar with a hand pump via the air valve |
| E-34 | Excessive cycling: more than thirty starts per hour (non-latched) | Check tank precharge and look for small leaks; the pump resumes after ten minutes |
| E-35 | Firmware checksum error (latched) | Switch off and on again; if the code persists the control board must be replaced by service |

If a code is not listed above, note the code and contact the dealer. Do not open the control board housing; it contains no user-serviceable parts.

## 7. Maintenance

The pump requires little maintenance when it is operated within its ratings. Carry out the tasks in the table below at the stated intervals. All tasks except the shaft seal inspection may be performed by the user.

| Interval | Task |
|----------|------|
| Weekly | Look for water drips at the pipe unions and at the drain plug |
| Monthly | Remove and rinse the inlet strainer; refit it the same way round |
| Every 3 months | Press RESET and MODE together to run the self-test; confirm that the display shows 00 |
| Every 6 months | Measure the tank precharge pressure with the pump switched off and depressurised; adjust it to 1.3 bar |
| Every 12 months | Check the fan cover and the ventilation slots for dust; clean with a dry brush |
| Every 2 years | Have the shaft seal inspected by an authorised service partner |
| Every 5 years | Replace the pressure tank diaphragm and the non-return valve gasket |

Use only water to clean the housing. Solvents and abrasive cleaners damage the display window.

If the pump is to be taken out of service for more than one month, drain it completely, remove the inlet strainer and store the pump in a dry room between 5 °C and 35 °C.

## 8. Technical data

| Parameter | Value |
|-----------|-------|
| Rated power input | 450 W |
| Supply voltage | 220–240 V, 50 Hz |
| Rated current | 2.1 A |
| Maximum flow rate | 3.2 m³/h |
| Maximum head | 32 m |
| Cut-in pressure | 1.5 bar |
| Cut-out pressure | 3.0 bar |
| Maximum system pressure | 6 bar |
| Maximum inlet pressure | 2.0 bar |
| Maximum suction lift | 8 m |
| Pressure tank volume | 2.0 litres |
| Tank precharge pressure | 1.3 bar |
| Inlet and outlet connection | G1 male thread |
| Water temperature range | 2 °C to 40 °C |
| Ambient temperature range | 4 °C to 40 °C |
| Protection class | IP44 |
| Insulation class | F |
| Sound pressure level | 58 dB(A) at 1 m |
| Net weight | 9.8 kg |
| Cable length | 1.5 m |
| Impeller material | Glass-fibre reinforced PPO |
| Pump body material | AISI 304 stainless steel |

The performance values apply to clean water at 20 °C and at the rated voltage. Flow and head decrease together along the pump curve; at 2.0 bar discharge pressure the P-450 delivers approximately 2.4 m³/h.

## 9. Warranty

The warranty period and the warranty conditions for the AquaBoost P-450 are set out in the separate document AquaBoost Warranty Terms (BWP-WT-01), which accompanies the invoice. This manual does not replace that document.

Keep the invoice and the serial number plate legible; both are required for any warranty claim. Modifications to the pump, use outside the ratings in section 8, and damage caused by frost, dry running or dirty water are not covered.

## 10. Disposal

At the end of its life the pump must not be disposed of with household waste. Take the pump to a collection point for electrical equipment, or return it to the dealer. The pressure tank must be depressurised through the air valve before disposal.

## 11. Declaration of conformity

Brightwater Pump Technologies Ltd. declares that the AquaBoost P-450 complies with the Machinery Directive 2006/42/EC, the Low Voltage Directive 2014/35/EU and the EMC Directive 2014/30/EU. The full declaration is available on request from the manufacturer.
