// A small requirement fixture exercising requirement, subject and satisfy.
package Demo {
	part def Drone;
	requirement <'REQ-1'> range {
		doc /* The drone shall fly far. */
		subject drone : Drone;
	}
	satisfy range by Drone;
}
