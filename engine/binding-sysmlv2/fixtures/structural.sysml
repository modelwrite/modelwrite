// A small structural fixture exercising the core subset.
package Demo {
	part def Engine {
		attribute power : Real;
	}
	part def Car {
		part eng : Engine;
		attribute speed = 0;
	}
	attribute def Color;
	part def SportsCar :> Car;
}
