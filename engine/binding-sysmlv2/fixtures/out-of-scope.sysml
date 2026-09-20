// A fixture whose constructs are OUTSIDE the subset: every one must be a
// named Unmappable loss, never a silent drop and never a panic.
package Demo {
	calc def MassSum {
		in masses : Real[0..*];
		return total : Real = sum(masses);
	}
	state def Machine {
		state idle;
		transition start first idle then running;
		state running;
	}
	actor Operator;
	usecase Operate;
	association Operates {
		memberEnd actor: Operator;
		memberEnd usecase: Operate;
	}
}
